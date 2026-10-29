# The yumete manual

The manual is now organized as an mdBook. Start at [the manual](en/index.md),
read the [contents](en/SUMMARY.md), or look up [help topics](en/help.md).

Build HTML with `python3 scripts/docs.py build`; add `--pdf` for PDF output.
Use `--backend default|mdbook-pandoc|mdbook-pdf|mdbook-typst` with `--pdf` to choose
a renderer; see [installation and configuration](en/documentation.md#pdf-backends).
The canonical English chapters live in `docs/en/`. Chinese sources use matching
paths in `docs/zh-Hant/`; run `python3 scripts/make_manual_sc.py` to generate
`docs/zh-Hans/`.
