<!-- help: 52-u-does-not-undo-nothing-happened -->

# `u` does not undo "nothing happened"

Recording an undo point when a command **announces** an edit, rather than when it
really moves a character, makes `u` "work sometimes" — three `d`s that deleted nothing
would leave three `u`s with nothing to undo, and `i` followed straight by `Esc` another
one. That is the fastest way there is to stop somebody trusting an editor.

So an undo point is **earned**: it is held when the edit is announced (ropey's
copy-shared structure, so holding an edit that never happened costs one pointer), and it
goes on the undo stack only once a character **really moved**. Same for an edit a guard
stopped — it moved no characters, so it is not a place worth going back to.


---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
