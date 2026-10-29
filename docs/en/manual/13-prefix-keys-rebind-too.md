<!-- help: 13-prefix-keys-rebind-too -->

# Prefix keys rebind too

The menu `空格` opens, and the groups opened by `g` `m` `t` — **write that prefix key
itself on the right** and the whole group moves. vim's hands like the leader on `;`,
with space kept for walking:

```toml
[keys.normal]
";" = "@ "      # semicolon opens that menu
" " = "@l"      # space moves one cell right
"<backspace>" = "@h"   # backspace moves left
```

**A prefix cannot be bound to an "action name"** — an action is one thing,
while a prefix is still waiting for a second key, and binding it that way swallows the
whole group. Binding it to a **key** always worked; the manual simply never said so.


---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
