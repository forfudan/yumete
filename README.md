# yumete · 宇夢編輯器

yumete is a terminal text editor made for the habits of people who write in
漢字, with both vim and helix keys. It moves the cursor by Chinese word, and
supports vertical editing, conversion between simplified and traditional
characters, pinyin search and more. It carries the
[Yume input engine](https://shurufa.app/docs/yume.html) inside it, so you can
type in the terminal with the scheme tables stored on your machine. Beyond
editing code, it makes reading and writing Chinese much easier.

宇夢編輯器是一款符合漢字文化圈用户習慣的終端文本編輯器，提供vim和helix兩種鍵位支持。它以**中文詞語**爲單位移動光標，支持竪排編輯、繁簡轉換、拼音搜索等常用功能，並内置[宇夢輸入系統核心引擎](https://shurufa.app/docs/yume.html)，允許你直接調用本地儲存的方案碼表進行TUI界面輸入。除了代碼編輯外，它爲中文文章的閲讀和寫作提供了極大的便利。

## Install

```sh
brew install forfudan/tap/yumete          # macOS and Linux
```

```powershell
scoop bucket add forfudan https://github.com/forfudan/scoop-bucket
scoop install yumete                      # Windows
```

Or take an archive from the [releases page](https://github.com/forfudan/yumete/releases).
It installs as `yumete`, and as `ye` for the one you type.

## Documentation

**<https://zhuyuhao.com/yumete-docs/>**, also as a
[book](https://zhuyuhao.com/yumete-docs/book/). What changed in each release is
in [CHANGELOG.md](CHANGELOG.md).

## Building from source

```sh
scripts/build.sh            # release build, linked as ~/.local/bin/yumete and ye
cargo test --workspace
```

The input method crate depends on the `yume` repository checked out beside this
one (`../yume`). See [Installation](https://zhuyuhao.com/yumete-docs/guide/installation)
for the options and for Windows.

## Licence

Apache License 2.0, see [LICENSE](LICENSE). The notices of the borrowed themes
are in [licenses/](licenses/).
