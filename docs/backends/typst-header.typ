#show raw: set text(font: (${TYPST_MONO_FONT}, ${CJK_FONT}))
#show raw.where(block: false): it => text(font: (${TYPST_MONO_FONT}, ${CJK_FONT}), size: 0.8em, it.text.replace(regex("[:/|.-]"), m => m.text + "\u{200b}"))
