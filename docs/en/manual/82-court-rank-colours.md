<!-- help: 82-court-rank-colours -->
<!-- help-index: options -->

# Court rank colours

Besides the gold and the red there are three more colours, for the things on the
page that are **not prose**. They are taken from the colours of official robes —
which **already have an order**, exactly right for saying how much authority a
piece of text carries:

| Rank | Colour | For | Why |
| --- | --- | --- | --- |
| Dragon robe | gold | headings | a writer's own voice |
| Imperial house | yellow | `::: warning` | one step deeper than the gold, and not the gold |
| Ranks 1–3 | purple | inline code, code blocks | the kind of literal where not one character may be wrong |
| Ranks 4–5 | red | footnotes, `::: caution`, conflicts | the red brush |
| Ranks 6–7 | green | quotations | somebody else's words |
| Ranks 8–9 | blue | links, `[[章節]]` | just an address |

**Bold, italic and headings get no hue** in the house themes — they are the writing
already, and colour on them is only noise. (The borrowed themes colour them the way the
theme they copy does.) But code is not prose; it is a run of literal text
quoted out of another language, and marking it with nothing but "one step darker
than the writing" says "this part does not matter", when it is often the most
precise thing in the line — and **it is the first thing to disappear when the
page goes darker** (the time the ink went from 13.7:1 to 10:1, inline code
against the paper fell from 7.3:1 to 5.5:1).

All three colours are **darker than the writing** (8.2–9.2:1 against the
writing's 11.1:1), so the writing is still the brightest thing on the page.

**Every colour has one variant**, five pairs, ten colours. The main one keeps
doing what it did; the variant is the secondary thing in the same family — and
for now the variants are used only in code colouring:

| Main | Variant | The variant is for |
| --- | --- | --- |
| gold | yellow | types (yellow was already there; it also does `::: warning`) |
| red | orange | numbers and constants in code |
| purple | pink | `self`, `this` |
| blue | cyan | escapes, `{…}` interpolation |
| green | yellow-green | HTML tags |

**Red is red; it does not lean orange.** Red has only ever said "this
is wrong"; if code wants an orange for its numbers, it gets an orange of its
own, not a loan from the red. In every theme that means its red to be red, the
red is corrected to a true red (OKLCH hue 29–30°, exactly the hue of pure red on
screen). `one-dark`, `github`, `monokai`, `dracula`, `tokyo-night`,
`catppuccin` and `gruvbox` take their colours, red included, from the themes
they copy.

**No ground out of the box**: the `` ` `` and the `>` are drawn there already,
the boundary is on the page, and laying a ground under it answers a question
that has been answered. If you want code to sit in a box, **`:theme-fill on`**
(`off` takes it back; with no argument it is the other way round).

**Five callouts, one colour each**, light to heavy — **GitHub's five alerts and
VitePress's five containers are the same set**, merged into one here:

| Callout | Colour | GitHub | VitePress |
| --- | --- | --- | --- |
| `::: note` | blue (ranks 8–9) | `[!NOTE]` | `info`, `details` |
| `::: tip` | green (ranks 6–7) | `[!TIP]` | `tip` |
| `::: important` | purple (ranks 1–3) | `[!IMPORTANT]` | — |
| `::: warning` | yellow (imperial house) | `[!WARNING]` | `warning` |
| `::: caution` | red | `[!CAUTION]` | `danger` |

`info`, `details`, `danger` and `error` are all recognised; whichever you write
lands in the same slot. **Yellow is not gold**: gold is the dragon robe and goes
to headings only; yellow is the imperial house, one step deeper than gold (six
degrees of hue apart, twenty-four lower in lightness).

Their grounds are like **a thin sheet of coloured glass laid on the paper**:
**keep the colour, move only the lightness** — toward the light in light mode,
toward the dark in dark mode — until it is **1.08:1** (light) / **1.12:1**
(dark) from the paper.

**The two knobs turn separately.** Pull the saturation down along with
it and you get grey (which is exactly how the first version was wrong); close
the distance without keeping the saturation and it is still that colour, only
thinner. The colour stays; it is just pale.

**Why the light-mode number is smaller**: the same ratio is the same
ΔL\* on both sides (measured: 1.5:1 is 15 either way), but **the weight is not
the same at all** — cutting light out of a bright sheet the eye has already
adapted to is not the same thing as adding a little light to a dark one.

**Yellow is the exception; it has to be more saturated than the
paper.** The light-mode paper is itself a warm cream at 45° and saturation 46,
and the yellow is at 38° — seven degrees of hue apart. With the saturation also
at 42 it would not be yellow, it would be "paper, a little darker than the
paper". So whichever colour is **in the paper's own family** (within thirty
degrees, and the paper itself has colour) gets its saturation lifted above the
paper's; the other four are more than a hundred degrees from the paper and do
not need this. One grey ground for all four would be the safer-looking
choice — "four grey grounds differ from each other by only 1.4–2.4 ΔE and cannot be
told apart" — but that argument is about **grey**. What separates these is hue, and how far each sits from the paper is set by the same one rule —
**distance says "this is a block", hue says "which block"** — and once each
thing does its own job, both hold.

**Red stays in the heaviest slot**, even though purple outranks it in
the robes. Red is the one colour in this editor with a hard meaning — merge
conflict markers, footnote numbers, rows whose column count is off all use it —
and two colours both saying "this is wrong" is the same as neither saying it.
Red is the most dangerous, and this is also the only convention nobody has to
learn.

`mode = "auto"` **asks the terminal** what its background is (OSC 11) and
decides light or dark from that — it asks the terminal rather than the desktop,
because somebody running a dark terminal on a light desktop has already answered
this question. If the terminal does not answer, it follows `dark`/`light` in the
config; writing `mode = "dark"` outright means it never asks.

`ground = "terminal"` gives the ground back to your terminal: yumete draws no
paper, and only uses the scale to decide how far each thing retreats. Good for
people who have tuned their own colours.


---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
