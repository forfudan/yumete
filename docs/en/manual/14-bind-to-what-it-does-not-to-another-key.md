<!-- help: 14-bind-to-what-it-does-not-to-another-key -->

# Bind to what it does, not to another key

A run of keys on the right means "pressing this counts as pressing that" — handy, but
you have to know first that `d` here is delete. **The right side can also be the name
of the thing itself**:

```toml
[keys.normal]
x = "delete_selection"      # an action name
"\\" = ":write"              # a command
"dj" = "@xxd"               # or a run of keys, with an @ to say so
```

Three kinds of right side, and **each says which it is**: a `:` in front is the command
line, an `@` in front is keys, and a bare word is an action's name. The good thing about
a name is that it does not move when the defaults move — the day `d` goes somewhere
else, `x = "delete_selection"` still deletes.

**A bare word that is not a name is refused**, and the line says so at start-up. That is
what the `@` is for: keys used to be whatever was left over, so `zz = "save"` — a
misspelling of `save_file` — was *pressed*, and `s` opened the search panel while `ave`
went into it. helix marks keys the same way and for the same reason.

**`:keymap-actions` lists them all** (eighty-odd), in three columns: name, what it
does, which key it is on now. The names match helix, so anyone coming from there can
guess — including the pairs helix spells with `_noyank`:

```toml
# What a helix discussion with ninety-odd votes asks for: delete and change
# stop filling the register, and the Alt keys do the cutting.
[keys.normal]
"A-d" = "delete_selection"
"d" = "delete_selection_noyank"
"A-c" = "change_selection"
"c" = "change_selection_noyank"
```

**There is a switch for exactly this, and it is the better way**:
`:keymap-yumete-flavor on` swaps the same two pairs, in a table's cells as well as
in prose, and `flavor = true` under `[keys]` makes it the default. Prefer it over
the four lines above: a **binding** goes on overriding whatever the defaults
become, so those four lines would quietly outlive the reason you wrote them.

The factory answer is off: `d` and `c` cut, as in helix and as in vi, and
`A-d`/`A-c` leave the register alone.

The flavour is a short list of departures laid over whichever preset is in use,
not a third preset — a preset is a whole table to keep level with helix's own,
and the majority of either keymap is not what is in dispute. Today the list holds
this one item. With no word, `:keymap-yumete-flavor` reports whether it is on and
what is in it, and says so when a `[keys.normal]` line of yours binds a key the
flavour would otherwise govern — that line wins, and the report names it.

A misspelt name is **said out loud on the spot** instead of being typed into
your text: a right side with an underscore in it that is not recognised is reported at
startup as "not an action name", and **that binding is not installed** — otherwise
pressing the key would type `delete_slection` into the manuscript one letter at a
time. (No key is spelled with an underscore, so an underscore means you meant a name.
Underscores inside a `:命令` do not count.)


---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
