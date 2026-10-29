<!-- Generated from docs/zh-Hant/help.md by scripts/make_manual_sc.py.
     请编辑繁体正本，不要直接修改此文件。 -->

<!-- help: help helphelp -->

# 查阅帮助

`:tutor` 打开可编辑的中文教程。`:help` 浏览完整手册。

| 查询 | 目的地 |
| --- | --- |
| `:help :write` | 命令说明；`:help :w` 也可用 |
| `:help 'editor.scrolloff'` | 配置选项及示例 |
| `:help 'scrolloff'` | 无歧义时可省略配置节名 |
| `:help w` | Normal 模式按键 |
| `:help i_<Esc>` | Insert 模式按键 |
| `:help v_;` | Select 模式按键 |
| `:help c_<Backspace>` | 命令行编辑按键 |

按键区分大小写。`<C-o>` 等同于 `C-o`；`<Space>` 表示空格前缀。

`gL` / `gd` 跟随文档链接；`C-o` 返回，`C-i` 前进。
默认键位参考 Helix，Vim 键位请查阅 [Vim preset](manual/12-vims-hands-keys-preset-vim.md)。

切换界面与帮助语言：`:language zhs`、`:language zh`、`:language en`。
指定帮助语言：`:help :write@en`、`:help help@zh-Hans`、`:help help@zh-Hant`。
缺少译文时会标明并显示英文原文。

[目录 / 目录](SUMMARY.md) · [English help](../en/help.html)
