#!/usr/bin/env python3
"""Check the shared manual sources; build multilingual mdBook HTML and PDF."""
import argparse
import html
import json
import shlex
import sys
try:
    import tomllib
except ModuleNotFoundError:
    raise SystemExit("Documentation builds require Python 3.11 or newer") from None
from html.parser import HTMLParser
from urllib.parse import unquote, urlsplit
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
DOCS = ROOT / 'docs'
BACKEND_DIR = DOCS / 'backends'
BACKENDS = ('default', 'mdbook-pandoc', 'mdbook-pdf', 'mdbook-typst')
LANGUAGES = {'en': 'English', 'zh-Hans': '简体中文', 'zh-Hant': '繁體中文'}
APPENDICES = {'project.md': ROOT / 'README.md', 'changelog.md': ROOT / 'CHANGELOG.md',
              'licenses.md': ROOT / 'licenses/README.md'}
LINK = re.compile(r'(?<!!)\[[^\]\n]*\]\(([^\s)]*)(?:\s+"[^"]*")?\)')


def without_code(text):
    text = re.sub(r'^(`{3,}|~{3,}).*?^\1[^\n]*$', '', text, flags=re.M | re.S)
    return re.sub(r'(`+).*?\1', '', text)


def slug(text):
    text = re.sub(r'<[^>]*>', '', text).replace('`', '')
    return re.sub(r'\s+', '-', ''.join(c for c in text.lower() if c.isalnum() or c in '_- '))


def anchors(text):
    prose = without_code(text)
    found = set(re.findall(r'<a\s+(?:id|name)=["\']([^"\']+)', prose))
    counts = {}
    for heading in re.findall(r'^#{1,6}\s+(.+)$', prose, re.M):
        base = slug(heading)
        count = counts.get(base, 0)
        found.add(base if count == 0 else f'{base}-{count}')
        counts[base] = count + 1
    return found


def pages():
    summary = (DOCS / 'en/SUMMARY.md').read_text(encoding='utf-8')
    return [m for m in re.findall(r'\]\(([^)]+)\)', summary) if m.endswith('.md')]


def source(lang, name):
    translated = DOCS / lang / name
    return translated if translated.is_file() else DOCS / 'en' / name


def check():
    subprocess.run([os.sys.executable, str(ROOT / "scripts/make_manual_sc.py"), "--check"], check=True)
    errors = []
    known = set(pages()) | {'SUMMARY.md'}
    for lang in LANGUAGES:
        for name in known - APPENDICES.keys():
            path = source(lang, name)
            if not path.is_file():
                errors.append(f'{lang}/{name}: missing source')
                continue
            text = path.read_text(encoding='utf-8')
            english = DOCS / 'en' / name
            if lang != 'en' and path != english:
                tags = lambda s: set(re.findall(r'<!-- help(?:-index)?: .*? -->', s))
                if not tags(english.read_text(encoding='utf-8')) <= tags(text):
                    errors.append(f'{lang}/{name}: missing English help tags')
            for target in LINK.findall(without_code(text)):
                if re.match(r'\w+:|//', target) or target.endswith('.html'):
                    continue
                dest, _, fragment = target.partition('#')
                relative = Path(os.path.normpath(str(Path(name).parent / dest))) if dest else Path(name)
                if str(relative) not in known:
                    errors.append(f'{lang}/{name}: unknown chapter {target}')
                elif fragment and str(relative) not in APPENDICES:
                    other = source(lang, str(relative))
                    if other.is_file() and fragment not in anchors(other.read_text(encoding='utf-8')):
                        errors.append(f'{lang}/{name}: unknown anchor {target}')
    if errors:
        raise SystemExit('\n'.join(errors))
    print(f'Checked {len(known)} pages in {len(LANGUAGES)} languages')


def chrome():
    candidates = [os.environ.get('CHROME'), shutil.which('chromium'), shutil.which('chromium-browser'),
                  shutil.which('google-chrome'), '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome']
    for base in ['PROGRAMFILES', 'PROGRAMFILES(X86)', 'LOCALAPPDATA']:
        if os.environ.get(base):
            candidates.append(str(Path(os.environ[base]) / 'Google/Chrome/Application/chrome.exe'))
    for candidate in candidates:
        if candidate and Path(candidate).is_file():
            return candidate
    raise SystemExit('PDF needs Chrome/Chromium; set CHROME to its executable')


def require_tool(name):
    path = shutil.which(name)
    if not path:
        raise SystemExit(f'Missing tool: {name}. See docs/en/documentation.md#pdf-backends')
    return path


def mdbook_driver(series):
    """Do not replace a user's default mdBook to accommodate one PDF plugin."""
    candidate = os.environ.get('MDBOOK')
    if not candidate:
        candidate = next((os.environ.get('MDBOOK_' + version.replace('.', ''))
                          for version in series if os.environ.get('MDBOOK_' + version.replace('.', ''))), 'mdbook')
    driver = require_tool(candidate)
    output = subprocess.run([driver, '--version'], check=True, capture_output=True, text=True).stdout
    match = re.search(r'mdbook\s+v?(\d+\.\d+)\.\d+', output)
    if not match or match[1] not in series:
        wanted = ' or '.join(series)
        variables = ', '.join('MDBOOK_' + v.replace('.', '') for v in series)
        raise SystemExit(f'This backend needs mdBook {wanted}.x; found {output.strip()!r}. '
                         f'Set {variables} (or MDBOOK) to a compatible executable. '
                         'See docs/en/documentation.md#pdf-backends')
    return driver


def backend_fonts(kind):
    profiles = tomllib.loads((BACKEND_DIR / 'fonts.toml').read_text(encoding='utf-8'))
    profile = os.environ.get('DOCS_FONT_PROFILE', 'macos' if sys.platform == 'darwin' else 'linux')
    if profile not in profiles:
        raise SystemExit(f'Unknown DOCS_FONT_PROFILE {profile!r}; choose from {", ".join(profiles)}')
    fonts = profiles[profile]
    wanted = ['main', 'sans', 'mono', 'cjk', 'symbols'] if kind == 'pandoc' else ['typst', 'typst-mono', 'cjk']
    if kind == 'typst':
        available = subprocess.run(['typst', 'fonts'], check=True, capture_output=True, text=True).stdout
        available = {name.strip().casefold() for name in available.splitlines()}
        missing = [fonts[key] for key in wanted if fonts[key].casefold() not in available]
    else:
        missing = []
        for key in wanted:
            matched = subprocess.run(['fc-match', '-f', '%{family}', fonts[key]], check=True,
                                     capture_output=True, text=True).stdout
            if fonts[key].casefold() not in {name.strip().casefold() for name in matched.split(',')}:
                missing.append(fonts[key])
    if missing:
        raise SystemExit(f'Missing PDF fonts: {", ".join(sorted(set(missing)))}. '
                         'Install the fonts or edit docs/backends/fonts.toml; '
                         'see docs/en/documentation.md#pdf-backends')
    return fonts


def prepare_backend(name='default', pdf=True):
    template = (BACKEND_DIR / f'{name}.toml').read_text(encoding='utf-8')
    config = tomllib.loads(template)
    metadata = config['backend']
    driver = mdbook_driver(metadata['mdbook-series'])
    values = {}
    if pdf:
        for tool in metadata.get('requires', []):
            require_tool(tool)
        if 'renderer' in metadata:
            values['RENDERER'] = shlex.quote(require_tool(metadata['renderer']))
        if name == 'default' or metadata.get('chrome'):
            values['CHROME'] = chrome()
        if 'fonts' in metadata:
            fonts = backend_fonts(metadata['fonts'])
            values.update({key.upper().replace('-', '_') + '_FONT': value for key, value in fonts.items()})
            if metadata['fonts'] == 'pandoc':
                header = (BACKEND_DIR / 'pandoc-header.tex').read_text(encoding='utf-8')
                values['PANDOC_HEADER'] = [header.replace('${SYMBOLS_FONT}', fonts['symbols'])]
            else:
                header = (BACKEND_DIR / 'typst-header.typ').read_text(encoding='utf-8')
                values['TYPST_HEADER'] = re.sub(r'\$\{([A-Z_]+)\}',
                                               lambda m: json.dumps(values[m[1]], ensure_ascii=False), header)
    output = template[template.index('\n[output.'):] if '\n[output.' in template else ''
    output = re.sub(r'"\$\{([A-Z_]+)\}"',
                    lambda m: json.dumps(values[m[1]], ensure_ascii=False), output)
    # Check expanded TOML now, before any output is replaced.
    tomllib.loads(output)
    return {'name': name, 'driver': driver, 'output': output, 'metadata': metadata}


def pdf_filename(backend):
    return 'yumete.pdf' if backend == 'default' else f'yumete-{backend}.pdf'


def validate_pdf(path):
    if not path.is_file():
        raise SystemExit(f'Backend did not produce a valid PDF: {path}')
    with path.open('rb') as pdf:
        if pdf.read(5) != b'%PDF-':
            raise SystemExit(f'Backend did not produce a valid PDF: {path}')


def install_book(stage, html_root, out, pdf_path=None, backend='default'):
    """Replace generated HTML only after a successful build; retain other PDFs."""
    retained = stage / 'retained-pdfs'
    retained.mkdir()
    if out.exists():
        for path in out.glob('*.pdf'):
            shutil.copy2(path, retained / path.name)
    if pdf_path:
        shutil.copy2(pdf_path, retained / pdf_filename(backend))
    # Assemble the replacement before removing the old generated book.
    replacement = stage / 'installed-book'
    shutil.copytree(html_root, replacement)
    for path in retained.iterdir():
        shutil.copy2(path, replacement / path.name)
    if out.exists():
        shutil.rmtree(out)
    out.parent.mkdir(parents=True, exist_ok=True)
    shutil.copytree(replacement, out)


def build(lang, pdf, backend='default', plan=None):
    plan = plan or prepare_backend(backend if pdf else 'default', pdf)

    out = DOCS / 'book' / lang
    with tempfile.TemporaryDirectory(prefix='yumete-book-') as temporary:
        stage = Path(temporary)
        # mdBook interprets MDBOOK_* as configuration overrides. These two
        # variables select our driver and must not reach it or its renderers.
        render_env = {key: value for key, value in os.environ.items()
                      if key not in {'MDBOOK_04', 'MDBOOK_05'}}
        src = stage / 'src'
        for name in pages() + ['SUMMARY.md']:
            dest = src / name
            dest.parent.mkdir(parents=True, exist_ok=True)
            if name in APPENDICES:
                text = APPENDICES[name].read_text(encoding='utf-8')
                if name == 'project.md':
                    text = text.replace('(docs/manual.md)', '(index.md)').replace('(docs/en/', '(').replace('(CHANGELOG.md)', '(changelog.md)')
                    text = text.replace('(LICENSE)', '(licenses.md)').replace('(licenses/)', '(licenses.md)')
                    text = text.replace('(crates/', '(https://github.com/forfudan/yumete/tree/master/crates/')
                elif name == 'licenses.md':
                    for notice in sorted((ROOT / 'licenses').glob('*.LICENSE')):
                        text += '\n## ' + notice.name + '\n\n```text\n' + notice.read_text(encoding='utf-8') + '\n```\n'
                    text += '\n## yumete license\n\n```text\n' + (ROOT / 'LICENSE').read_text(encoding='utf-8') + '\n```\n'
            else:
                path = source(lang, name)
                text = path.read_text(encoding='utf-8')
                if lang != 'en' and path == DOCS / 'en' / name and name != 'SUMMARY.md':
                    notice = '本页尚未翻译，以下为英文原文。' if lang == 'zh-Hans' else '本頁尚未翻譯，以下爲英文原文。'
                    text = f'> {notice}\n\n' + text
            dest.write_text(text, encoding='utf-8')
        shutil.copy(DOCS / 'theme/print.css', stage / 'print.css')
        (stage / 'book.toml').write_text(f'''[book]
title = "yumete · {LANGUAGES[lang]}"
language = "{lang}"
src = "src"

[build]
build-dir = "out"

[output.html]
additional-css = ["print.css"]
site-url = "/{lang}/"

[output.html.print]
enable = true
''', encoding='utf-8')
        if pdf and backend == 'mdbook-pdf':
            # mdbook-pdf rewrites print.html with hidden helper links. Retain a
            # clean HTML render rather than publishing the plugin's mutations.
            subprocess.run([plan['driver'], 'build', str(stage)], check=True, env=render_env)
            shutil.copytree(stage / 'out', stage / 'clean-html')
        if pdf:
            with (stage / 'book.toml').open('a', encoding='utf-8') as config:
                config.write(plan['output'])
        subprocess.run([plan['driver'], 'build', str(stage)], check=True, env=render_env)
        result_root = stage / 'out'
        html_root = result_root / 'html' if pdf and backend != 'default' else result_root
        if (stage / 'clean-html').exists():
            html_root = stage / 'clean-html'
        pdf_path = None
        if pdf:
            if backend == 'default':
                pdf_path = stage / 'yumete.pdf'
                print_pdf(html_root / 'print.html', pdf_path)
            else:
                pdf_path = result_root / plan['metadata']['result-path']
            validate_pdf(pdf_path)
        install_book(stage, html_root, out, pdf_path, backend)
    if pdf:
        print(out / pdf_filename(backend))


def print_pdf(html_path, pdf_path):
    with tempfile.TemporaryDirectory(prefix='yumete-chrome-') as profile:
        arguments = tomllib.loads((BACKEND_DIR / 'default.toml').read_text(encoding='utf-8'))['chrome']['arguments']
        subprocess.run([chrome(), *arguments, f'--user-data-dir={profile}',
                        f'--print-to-pdf={pdf_path.resolve()}',
                        html_path.resolve().as_uri()],
                       check=True, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    if not pdf_path.is_file():
        raise SystemExit(f'PDF not produced: {pdf_path}')
    # Chrome exposes outlines in an uncompressed catalog dictionary. Fail early
    # if an older browser silently ignores the outline switch.
    if not re.search(rb'/Type\s*/Outlines\b', pdf_path.read_bytes()):
        raise SystemExit('PDF has no bookmark outline; update Chrome/Chromium and rebuild')



def check_html(languages):
    """Check rendered links too: include expansion and mdBook can change paths."""
    class Links(HTMLParser):
        def __init__(self):
            super().__init__()
            self.links = []
            self.ids = set()

        def handle_starttag(self, tag, attrs):
            attrs = dict(attrs)
            if 'id' in attrs:
                self.ids.add(attrs['id'])
            if tag == 'a' and 'href' in attrs:
                self.links.append(attrs['href'])

    base = (DOCS / 'book').resolve()
    parsed = {}
    for lang in languages:
        for path in (base / lang).rglob('*.html'):
            links = Links()
            links.feed(path.read_text(encoding='utf-8'))
            parsed[path] = links
    errors = []
    for path, page in parsed.items():
        for link in page.links:
            url = urlsplit(link)
            if url.scheme or url.netloc or url.path.startswith('/'):
                continue
            dest = (path.parent / unquote(url.path)).resolve() if url.path else path
            try:
                relative = dest.relative_to(base)
            except ValueError:
                errors.append(f'{path.relative_to(base)}: link escapes book: {link}')
                continue
            if relative.parts[0] in LANGUAGES and relative.parts[0] not in languages:
                continue
            if not dest.exists():
                errors.append(f'{path.relative_to(base)}: missing {link}')
            elif url.fragment and dest in parsed and unquote(url.fragment) not in parsed[dest].ids:
                errors.append(f'{path.relative_to(base)}: missing anchor {link}')
    if errors:
        raise SystemExit('\n'.join(sorted(set(errors))))
    print(f'Checked links and anchors in {len(parsed)} HTML files')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=['check', 'build'])
    parser.add_argument('--lang', choices=list(LANGUAGES))
    parser.add_argument('--pdf', action='store_true')
    parser.add_argument('--backend', choices=BACKENDS, help='PDF backend (default: default); requires build --pdf')
    args = parser.parse_args()
    if (args.pdf or args.backend is not None) and args.command != 'build':
        parser.error('--pdf and --backend require the build command')
    if args.backend is not None and not args.pdf:
        parser.error('--backend requires --pdf')
    backend = args.backend or 'default'
    plan = prepare_backend(backend if args.pdf else 'default', args.pdf) if args.command == 'build' else None
    check()
    if args.command == 'build':
        languages = [args.lang] if args.lang else list(LANGUAGES)
        for lang in languages:
            build(lang, args.pdf, backend, plan)
        check_html(languages)
        landing = DOCS / 'book/index.html'
        landing.write_text('<!doctype html><html lang="en"><meta charset="utf-8"><title>yumete manuals</title>'
                           '<h1>yumete</h1><ul>' + ''.join(f'<li><a href="{lang}/index.html">{html.escape(label)}</a></li>'
                           for lang, label in LANGUAGES.items()) + '</ul></html>', encoding='utf-8')


if __name__ == '__main__':
    main()
