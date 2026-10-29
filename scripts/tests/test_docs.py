"""Protect document identifiers and source checks, not generated prose layout."""
import importlib.util
from pathlib import Path
import unittest
import tempfile
import os
import subprocess
import tomllib
from types import SimpleNamespace
from unittest.mock import patch

SCRIPTS = Path(__file__).resolve().parents[1]


def module(name):
    spec = importlib.util.spec_from_file_location(name, SCRIPTS / f'{name}.py')
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


class DocumentationTests(unittest.TestCase):
    def test_conversion_preserves_identifiers_and_character_examples(self):
        convert = module('make_manual_sc').convert
        source = ('<!-- help: 縱書 -->\n[縱書](manual/25-vertical-layout-縱書.md#縱書)\n'
                  '<a id="縱書"></a>\n選單 裏爲説\n'
                  '<!-- verbatim -->臺灣 裏 為<!-- verbatim -->\n')
        result = convert(source)
        self.assertIn('[纵书](manual/25-vertical-layout-縱書.md#縱書)', result)
        self.assertIn('<!-- help: 縱書 -->', result)
        self.assertIn('<a id="縱書"></a>', result)
        self.assertIn('菜单 里为说', result)
        self.assertIn('<!-- verbatim -->臺灣 裏 為<!-- verbatim -->', result)

    def test_unbalanced_verbatim_fails_without_output(self):
        with self.assertRaises(ValueError):
            module('make_manual_sc').convert('before <!-- verbatim --> after')

    def test_pdf_contains_native_bookmark_outline(self):
        docs = module('docs')
        try:
            docs.chrome()
        except SystemExit:
            self.skipTest('Chrome/Chromium is required for the PDF integration test')
        with tempfile.TemporaryDirectory(prefix='yumete-test-outline-') as temporary:
            folder = Path(temporary)
            source, output = folder / 'book.html', folder / 'book.pdf'
            source.write_text('<!doctype html><meta charset="utf-8">'
                              '<h1>Manual</h1><h2>中文章节</h2><p>Chapter text.</p>')
            docs.print_pdf(source, output)
            data = output.read_bytes()
            self.assertRegex(data, rb'/Type\s*/Outlines\b')
            self.assertRegex(data, rb'/Title\s*\(Manual\)')
            self.assertIn(b'/Dest', data)

    def test_checker_ignores_examples_but_keeps_real_links(self):
        docs = module('docs')
        text = '```md\n[example](missing.md)\n```\n`[inline](missing.md)`\n[real](help.md)'
        self.assertEqual(docs.LINK.findall(docs.without_code(text)), ['help.md'])


class BackendTests(unittest.TestCase):
    def test_cli_requires_pdf_for_backend(self):
        for args in [['build', '--backend', 'mdbook-pandoc'], ['check', '--pdf']]:
            with self.subTest(args=args), patch('sys.argv', ['docs.py', *args]):
                with self.assertRaises(SystemExit) as error:
                    module('docs').main()
                self.assertEqual(error.exception.code, 2)

    def test_driver_version_mismatch_explains_side_by_side_install(self):
        docs = module('docs')
        with patch.dict(os.environ, {}, clear=True), \
             patch.object(docs, 'require_tool', return_value='/tools/mdbook'), \
             patch.object(docs.subprocess, 'run', return_value=SimpleNamespace(stdout='mdbook v0.4.52\n')):
            with self.assertRaisesRegex(SystemExit, 'MDBOOK_05'):
                docs.mdbook_driver(['0.5'])

    def test_driver_can_use_independent_version(self):
        docs = module('docs')
        with patch.dict(os.environ, {'MDBOOK_05': '/tools/mdbook05'}, clear=True), \
             patch.object(docs, 'require_tool', side_effect=lambda name: name), \
             patch.object(docs.subprocess, 'run', return_value=SimpleNamespace(stdout='mdbook v0.5.2\n')):
            self.assertEqual(docs.mdbook_driver(['0.5']), '/tools/mdbook05')

    def test_all_backend_templates_expand_to_valid_native_config(self):
        docs = module('docs')
        profiles = tomllib.loads((docs.BACKEND_DIR / 'fonts.toml').read_text(encoding='utf-8'))
        for profile in profiles.values():
            for backend in docs.BACKENDS:
                with self.subTest(backend=backend), \
                     patch.object(docs, 'mdbook_driver', return_value='/tools/mdbook'), \
                     patch.object(docs, 'require_tool', side_effect=lambda name: '/tools/' + name), \
                     patch.object(docs, 'chrome', return_value='/Applications/Chrome Browser'), \
                     patch.object(docs, 'backend_fonts', return_value=profile):
                    plan = docs.prepare_backend(backend)
                    config = tomllib.loads(plan['output'])
                    self.assertNotIn('${', plan['output'])
                    self.assertNotIn('backend', config)
                    if backend != 'default':
                        renderer = backend.removeprefix('mdbook-')
                        self.assertIn('command', config['output'][renderer])
                        if backend == 'mdbook-typst':
                            self.assertEqual(config['output']['typst']['output']['name'], 'yumete.pdf')
                        if backend == 'mdbook-pandoc':
                            variables = config['output']['pandoc']['profile']['pdf']['variables']
                            self.assertIn(profile['symbols'], variables['header-includes'][0])

    def test_html_build_does_not_require_pdf_tools(self):
        docs = module('docs')
        with patch.object(docs, 'mdbook_driver', return_value='/tools/mdbook'), \
             patch.object(docs, 'require_tool') as tool, patch.object(docs, 'chrome') as chrome, \
             patch.object(docs, 'backend_fonts') as fonts:
            docs.prepare_backend('default', pdf=False)
            tool.assert_not_called()
            chrome.assert_not_called()
            fonts.assert_not_called()

    def test_missing_renderer_is_reported_before_build(self):
        docs = module('docs')
        with patch.object(docs, 'mdbook_driver', return_value='/tools/mdbook'), \
             patch.object(docs.shutil, 'which', return_value=None):
            with self.assertRaisesRegex(SystemExit, 'Missing tool: mdbook-pdf'):
                docs.prepare_backend('mdbook-pdf')

    def test_missing_font_is_reported(self):
        docs = module('docs')
        with patch.dict(os.environ, {'DOCS_FONT_PROFILE': 'linux'}), \
             patch.object(docs.subprocess, 'run', return_value=SimpleNamespace(stdout='Arial')):
            with self.assertRaisesRegex(SystemExit, 'Missing PDF fonts:'):
                docs.backend_fonts('pandoc')

    def test_switching_backends_preserves_pdfs_and_failed_build_keeps_output(self):
        docs = module('docs')
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / 'en/index.md'
            source.parent.mkdir()
            source.write_text('# Manual\n', encoding='utf-8')
            (source.parent / 'SUMMARY.md').write_text('- [Manual](index.md)\n', encoding='utf-8')
            (root / 'theme').mkdir()
            (root / 'theme/print.css').write_text('')
            current = 'default'

            def render(cmd, **kwargs):
                self.assertNotIn('MDBOOK_04', kwargs['env'])
                self.assertNotIn('MDBOOK_05', kwargs['env'])
                stage = Path(cmd[-1])
                config = tomllib.loads((stage / 'book.toml').read_text(encoding='utf-8'))
                renderers = config['output']
                result = stage / 'out'
                html = result / 'html' if len(renderers) > 1 else result
                html.mkdir(parents=True)
                (html / 'index.html').write_text('fresh ' + current)
                (html / 'print.html').write_text('plugin-mutated' if 'pdf' in renderers else 'print')
                for renderer, relative in [('pandoc', 'pandoc/pdf/yumete.pdf'),
                                            ('pdf', 'pdf/output.pdf'), ('typst', 'typst/yumete.pdf')]:
                    if renderer in renderers:
                        dest = result / relative
                        dest.parent.mkdir(parents=True, exist_ok=True)
                        dest.write_bytes(b'%PDF-' + current.encode())

            def print_pdf(source, dest):
                dest.write_bytes(b'%PDF-default')

            with patch.dict(os.environ, {'MDBOOK_04': '/tools/mdbook04', 'MDBOOK_05': '/tools/mdbook05'}), \
                 patch.object(docs, 'DOCS', root), patch.object(docs, 'pages', return_value=['index.md']), \
                 patch.object(docs, 'mdbook_driver', return_value='/tools/mdbook'), \
                 patch.object(docs, 'require_tool', side_effect=lambda name: '/tools/' + name), \
                 patch.object(docs, 'chrome', return_value='/tools/chrome'), \
                 patch.object(docs, 'backend_fonts', return_value=tomllib.loads(
                     (docs.BACKEND_DIR / 'fonts.toml').read_text(encoding='utf-8'))['macos']), \
                 patch.object(docs, 'print_pdf', side_effect=print_pdf), \
                 patch.object(docs.subprocess, 'run', side_effect=render):
                for current in docs.BACKENDS:
                    docs.build('en', True, current)
                    if current == 'mdbook-pdf':
                        self.assertEqual((root / 'book/en/print.html').read_text(), 'print')
                out = root / 'book/en'
                self.assertEqual(len(list(out.glob('*.pdf'))), 4)
                for backend in docs.BACKENDS:
                    self.assertEqual((out / docs.pdf_filename(backend)).read_bytes(), b'%PDF-' + backend.encode())
                # A plain HTML rebuild removes stale HTML, retaining all four PDFs.
                (out / 'stale.html').write_text('obsolete')
                docs.build('en', False)
                self.assertFalse((out / 'stale.html').exists())
                before = (out / 'index.html').read_text()
                with patch.object(docs.subprocess, 'run', side_effect=subprocess.CalledProcessError(1, 'mdbook')):
                    with self.assertRaises(subprocess.CalledProcessError):
                        docs.build('en', True, 'mdbook-pdf')
                self.assertEqual((out / 'index.html').read_text(), before)
                self.assertEqual(len(list(out.glob('*.pdf'))), 4)

    def test_renderer_must_produce_an_actual_pdf(self):
        docs = module('docs')
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / 'output.pdf'
            path.write_text('renderer failed')
            with self.assertRaisesRegex(SystemExit, 'valid PDF'):
                docs.validate_pdf(path)


if __name__ == '__main__':
    unittest.main()
