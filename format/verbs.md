# Verbs

A **verb** is a named sequence of steps a map exposes as its public
interface (design §11, D7; V1-D3). Its steps start and stop the map's
presentations, apply the map's edits, wait, and run the map's other
verbs. A node's click and hover, a span's link, the console, IPC and the
user's and the app's macros invoke them; nothing else in a map does
anything. Where the console and `mandalactl` also have "verbs" of their
own, this page and the design say **map verb**.

```json
{ "id": "reveal-chapter-2", "name": "Reveal chapter 2",
  "description": "Brings the second chapter in, then lays it out.",
  "steps": [ { "present": "fade-in", "on": { "nodes": [12, 13] } },
             { "wait": 300 },
             { "present": "glow", "on": { "channel": 2, "within": "self" } },
             { "edit": "flower-layout", "on": "self" },
             { "stop": "glow", "on": "self" } ] }
```

**A verb is sandboxed** (§11.3). Its steps are the five below and
nothing else: no step runs a console line, an Action, or anything that
touches a file or the process. A map someone sent you can make itself
move and lay itself out; it cannot save, open, quit or run a command.
Verbs replace the map-level and node-level macro tiers, which could;
the user's and the app's macros remain the user's own automation, and
can run a verb ([`macros.md`](./macros.md)).

## Where verbs come from

From the map alone: its top-level `verbs` list
([`schema.md`](./schema.md#top-level)). A verb names the map's **own**
presentations and edits, by id (amendment 2 on #252), and its own verbs;
an edit may be a handler edit, which runs a built-in handler. The App
and User tiers of edits and presentations, which the console reaches by
id, are not a verb's to name: a map that names one does not load.

## The definition

### Verb

| Key | Type | Required | Default | Meaning |
|---|---|---|---|---|
| `id` | string | yes | | The id triggers, links, the console and IPC name it by. Not empty, no control or bidi characters; unique within `verbs`. |
| `name` | string | yes | | The name shown to the user. |
| `description` | string | no | `""` | What it does, for `verb list`. |
| `steps` | array of [steps](#step) | yes | | What it does, in order. |

### Step

Exactly one of `present`, `stop`, `edit`, `wait` and `verb`; `on`
beside the first three and nowhere else. Any other key is refused.

| Key | Type | Required | Default | Meaning |
|---|---|---|---|---|
| `present` | string | one of five | | Start the map's presentation with this id on the targets ([`presentations.md`](./presentations.md)). Started on N nodes it is N instances; one already playing on a node restarts there. |
| `stop` | string | one of five | | Stop the map's presentation with this id on the targets: each instance there plays its way out. |
| `edit` | string | one of five | | Apply the map's edit with this id at the targets ([`edits.md`](./edits.md)): one history entry, recorded as the verb's (`ChangeSource::Verb`), animated if the edit is. |
| `wait` | integer | one of five | | Wait this many milliseconds on the app clock, `0` to `60000`. |
| `verb` | string | one of five | | Run the map's verb with this id, inline: its steps, then the step after this one. |
| `on` | [target](#target) | with `present`, `stop`, `edit` | | Which nodes the step acts on. |

### Target

`on` is one of these spellings:

| Spelling | The nodes |
|---|---|
| `"self"` | the node the verb was invoked on |
| `"children"` | the child nodes of `self` |
| `"descendants"` | every node below `self` |
| `"selection"` | what is selected when the step runs |
| `{ "nodes": [ids] }` | these nodes, by id |
| `{ "channel": n, "within": "self" }` | the nodes below `self` on channel `n` |
| `{ "channel": n, "within": "map" }` | every node of the map on channel `n` |

Every target is resolved **when its step runs**, against the map as it
stands then: a node an earlier step or an edit added, moved or removed
is where it now is. The tree-walking targets list their nodes in tree
order; `nodes` and `selection` in their own order.

**A node a target names by id is not checked at load (D27).** One the
map does not have when the step runs is skipped, with a warning in the
log, and the step acts on the target's other nodes. Deleting a node a
verb names is an ordinary delete, and the map still loads. Its id is
never given to a new node while the verb names it (D32,
[`schema.md`](./schema.md)): the id counter starts above every id a
target names.

A verb that acts on `self` — its own step, or a step of a verb it runs,
targets `self`, `children`, `descendants` or a channel `within: "self"`
— needs a node to be run on. Invoked without one (from the console or
IPC with no node), it is refused, with a message, and nothing runs.

## Running

Running a verb makes a **run**: the verb, the node it was invoked on,
where it is in its steps, the time it has reached on the app clock, and
the presentation instances it started. Several runs may be active at
once, each going on with its own steps.

- `present`, `stop`, `edit` and `verb` steps run **at once, in order**,
  when the run reaches them. `wait` suspends the run until the app
  clock reaches its end; the frame then runs the steps after it.
- **A run keeps time by its own steps.** A `wait` moves the run's time
  on by its milliseconds, and the steps after it run at that time — a
  presentation starts there, an animated edit starts there — whenever
  the frame that resumes it comes. On a virtual clock (headless, IPC
  `clock.step`) a run is deterministic: stepped by 300 ms, a `wait: 300`
  ends on the step's last frame and its next presentation is at `t`
  exactly as if it had started on the instant.
- **A verb never settles an animated edit to start a presentation.** A
  `present` or `stop` step leaves an edit that is animating as it is;
  a hover must not cut short the change the user is watching. An `edit`
  step is an apply, and so does what every apply does (D28): an
  animated edit still running is finished and committed first. While
  an interactive transaction is open — a text editor, a drag, a
  preview — an `edit` step is skipped, with a warning in the log, and
  the run goes on.
- A presentation a step starts while the presentation layer is set
  aside answers its start, and any refusal of the platform's bounds
  arrives later, on the instance, when the layer is put back
  ([`presentations.md`](./presentations.md#the-document-layer)): a
  verb relies on a start's answer for nothing but the instances it
  names.

### Held runs

A hover's run is **held**. When the pointer leaves the node, the run is
**released**: the steps it has not run are skipped, and every `while`
instance it started is stopped, so each plays its way out. Instances of
the other lifecycles it started play on as their `play` says — a `hold`
until something stops it, a `loop` until it is stopped, a `once` to its
end. A run that is not held ends with its last step; its `while`
instances hold until a step or the console stops them.

## Triggers

| Trigger | Invokes |
|---|---|
| A click on a node | its `on.click` verb, with `self` the node |
| The pointer resting on a node | its `on.hover` verb, held, with `self` the node; leaving the node, or the window, releases it |
| A click on a span with a `link` | `{ "verb": id }`: that verb, with `self` the node the span belongs to; `{ "url": … }`: opens the address |
| The console | `verb run <id> [node-id]` |
| IPC | [`act.verb`](./ipc.md#actverb), and `mandalactl act.verb <id> [node-id]` |
| A user or app macro | its `Verb` step ([`macros.md`](./macros.md)) |

- **A click selects as it always does.** A node's click verb runs
  first, then the selection updates as for any click.
- **A linked span is the click's target before its node** (ruling D3.1
  on #252): a click on a span with a `link` runs the link, and not also
  the node's `on.click`; a click elsewhere on the node runs the node's.
- **Hover follows the pointer.** Moving from one node to another
  releases the first before invoking the second, and the pointer
  leaving the window (the canvas, in the browser) releases the hover as
  leaving the node does (ruling D3.2 item 4 on #252). During a gesture
  — a press not yet released — the hover stays as it was, and a gesture
  that ends with the pointer outside releases it then (ruling D3.3
  item 4). Touch has no hover: a tap is a click.
- **A hover is decided by the pointer's moves, and only by them**
  (ruling D3.2 item 4). When the scene changes under a pointer that
  does not move — a click's verb hides or moves the node it rests on,
  a presentation slides another under it — the hover is not decided
  again: the run it holds stays held until the pointer next moves or
  leaves the window.
- **The press decides what a click triggers** (ruling D3.3 item 5):
  a click triggers what its press landed on, read on the map as it
  stands at the release. When that is gone by then — the span, the
  connector, the link or the node removed in between — the click is
  on nothing and triggers nothing: not the node's `on.click` in its
  place.
- A node is the node a click on it selects: a click or a hover on a
  node's connector or one of its links is not on the node.
- The pointer shows a hand over a node with an `on.click` and over a
  linked span, on both targets.

### Links

A span link's `url` opens in the platform's handler for it — the
default browser, or the mail client for `mailto:` — through the
platform's opener on native and in a new tab (`window.open(url,
"_blank", "noopener,noreferrer")`) in the browser, so the page a map
links to gets no handle on Mandala's tab (ruling D3.2 item 1 on #252).
**Only `http`, `https` and `mailto` addresses** (in any case) are
accepted: any other scheme — `file`, `javascript`, `data`, a custom
handler — is refused at load, at the link's `url` (`LinkScheme`).
**A `mailto` address names only RFC 6068's headers** — `to`, `cc`,
`bcc`, `subject`, `body` and `in-reply-to` (ruling D3.3 item 1, which
supersedes D3.2 item 2's attachment denylist). Some mail clients have
attached a local file an `attach` or `attachment` header named, and
clients read a query differently, so any other name is refused, however
it is spelled. The whole address is split on one set of separators —
`?`, `&`, `;`, `,` and `#` — the query (what follows the first raw `?`)
and the address part before it alike (rulings D3.4 item 1 and D3.5
item 1), since a client may split what follows `mailto:` without first
finding the `?`; and it is read two ways, both of which must pass:

- **split first**: each part's name — what it writes before its first
  `=` — is percent-decoded once (`+` stays `+`) and its ASCII letters
  lowercased, and must be one of the six exactly. Nothing is trimmed:
  `attach+`, `attach%00` and ` attach` are refused. In the query, a part
  after a `?`, `&` or `;` is a header, so all of it is its name when it
  has no `=` (`?attach` is refused); a part after a `,` or a `#` is a
  header only when it holds an `=`, so a fragment (`?to=c@d#frag`) loads
  and `?subject=hi#attach=…` does not. In the address part every part
  that holds an `=` is a header, the first one too: `mailto:attach=…`
  is refused, `mailto:to=a@b` loads;
- **decode first**: percent-decode the query once, reading `+` as a
  space, and the address part once, then split the same way; every
  part with an `=` must name one of the six before it, lowercased — in
  the address part every part after the first, so `mailto:a%3Db@c`
  loads. A part with no `=` is text, so a body `Q%26A` loads.

**Control characters** — C0 (line breaks and tabs among them), DEL, C1
(NEL among them), and the Unicode line and paragraph separators U+2028
and U+2029 — are refused **raw anywhere** in a `mailto`: a link writes
them encoded (RFC 6068), and nothing else in a map refuses one in a
`url`. **Decoded**, one may stand only **inside a `body`**, and only a
line break (`%0D`, `%0A`) or a tab (`%09`): a client that writes the
message out as text would read what follows a break in any other header
as a header line of its own (`subject=x%0D%0ABcc:%20…`), and the other
controls have no place in a header either (ruling D3.5 item 3 and
verification round 5). The tab is allowed in a `body` because it is
ordinary text there — indentation in a message — and breaks no line;
anywhere else it is refused with the rest.

*Inside a `body`* is decided under every split a client may make at
once: under RFC 6068's (the query split on raw `&` alone, each value
decoded once), under the decode-first reading's, and so under every
split between them. A decoded control is allowed only when, within its
`&`-separated header, every part from the header's first up to the one
holding the control, split on all five separators once decoded, holds
an `=` and is named `body`. So `subject=x%26body=y%0D%0ABcc:%20…` is
refused, though the decode-first reading alone sees the break in a
`body`: under RFC 6068's split it is in the `subject`.

An address part that decodes to hold a `?` (`a@b%3Fattach=…`) is
refused, since a client may read the rest as a query, and no address
part is a `body`, so a control decoded there is refused too. A refused
address is refused at its `url`: `LinkMailHeader`, naming the header as
written or as it reads decoded, or `LinkMailControl`, saying what it
found — "a raw control character", "a decoded line break", "a decoded
control character" — and the part that holds it.

**What is over-refused**, and is kept so (rulings D3.4 item 2 and
D3.5):

- **a `body` or `subject` holding a web address with a query string**,
  as a "share this page" link writes one —
  `body=see%20https%3A%2F%2Fx.org%2F%3Fq%3D1` reads a header `q` once
  decoded. Encoding does not help, since the decode-first reading
  decodes it: the only way around it is to leave the query out of the
  web address;
- any other value that decodes to look like a header —
  `body=a%26b%3Dc` reads a header `b`;
- a value holding a `,` or a `#` before an `=` — `body=a,b=c` reads a
  header `b`, `subject=x#y=z` a header `y`;
- a raw `;` or `?` in a value, which the split-first reading takes for
  a part with a name: encode it, or leave it out;
- a decoded control in a value that is not the `body`;
- **a line break in a `body` after any `?`, `&`, `;`, `,` or `#` in
  it, raw or encoded** — `body=Hi%20Bob,%0D%0AThanks`,
  `body=How%20are%20you%3F%0D%0ABest`: split there, the break stands in
  a part with no `=`, which some client may begin a header with. That
  is most multi-line bodies; to keep the break, leave the separator out
  of the text before it;
- **an address after the first whose local part holds an `=`, raw or
  encoded** — `a@b;c=d`, `a@b,x=y@z`, `x@y,a%3Db@c` (a header `a`).
  Encoding does not help, since the decode-first reading decodes it:
  put that address first, or leave it out.

A `set_link` an edit or a presentation writes to either kind of
address is refused when the edit or the presentation is read, and
again by the writer when it is applied, and the app's opener refuses
either before it hands an address on, so no map can come to hold one
or open one.

## Load checks

A map's verbs are read after its edits and presentations. Refused, at
the path of the offending key:

- a step with none of the five step keys, with two of them, with any
  other key, with `on` beside `wait` or `verb`, or without `on` beside
  `present`, `stop` or `edit` — the sandbox;
- a target in no spelling of the [table](#target), or a node id in it
  that no node can carry (`0`, or past `4294967294`);
- a `present` or `stop` naming a presentation the map does not define,
  an `edit` naming an edit it does not define, a `verb` naming a verb it
  does not define;
- a `wait` past 60,000 ms;
- a cycle of `verb` calls, at the step that closes it, naming the verbs
  of the cycle;
- a chain of calls more than **8** verbs long (a verb that calls none is
  one), at the verb at its head;
- a verb of more than **65,536 parts** once the verbs it runs are
  expanded, at the verb.

## Bounds

A verb's **parts** are its steps and the node ids its targets name,
and those of every verb it runs, as often as it runs them (ruling D3.1
on #252; the owner's definition bound, format Q4: parts are counted after
any expansion the format allows). At most **65,536**. A verb that runs
another sixteen times, which runs another sixteen times, and so on four
deep, is a few hundred bytes of text and 69,904 parts, and is refused; so
one invocation runs at most 65,536 steps, however its calls fan out.

A `wait` is at most **60,000** ms (ruling D3.1), the bound of an edit's
and a presentation's animation: while a run waits the app keeps drawing
frames, and a number from a shared map must not hold it there for
longer than a person waits for one step of a map. Calls nest at most
**8** verbs deep (#252 §1).

## On the surfaces

- Console: `verb list` — each verb by id and name, marked **changes the
  map** when it has an `edit` step (its own, or one of a verb it runs)
  and **performance** otherwise (design §11.1), then the runs active —
  and `verb run <id> [node-id]`.
- IPC: [`act.verb`](./ipc.md#actverb) runs one, and its reply says
  whether it changes the map; [`query.animations`](./ipc.md#queryanimations)
  lists the runs active, and `clock.wait until=animations_complete`
  waits for a run that waits.
- `mandalactl act.verb <id> [node-id]` (ruling D3.1).
- Macros: `{ "kind": "Verb", "id": "…" }`, on the selected node or on
  `{ "target": { "node_id": n } }` ([`macros.md`](./macros.md)).

## Example: a sequential reveal

A map that expands for the viewer (D17) is two verbs and one
presentation:

```json
"presentations": [
  { "id": "concealed", "name": "Concealed", "play": "hold", "target": "node",
    "mutations": [ { "node": { "set_hidden": true } }, { "node": { "set_opacity": 0.0 } } ],
    "return": { "ms": 400 } }
],
"verbs": [
  { "id": "begin", "name": "Begin", "steps": [ { "present": "concealed", "on": { "channel": 1, "within": "map" } } ] },
  { "id": "next-1", "name": "Next", "steps": [ { "stop": "concealed", "on": { "nodes": [4] } } ] }
]
```

`begin` conceals every channel-1 node; each `next-*` stops the
presentation on one node, which plays its `return`: shown at once, its
opacity rising from 0 to 1 over 400 ms. The saved map always shows every
node. `src/application/app/dispatch/families/fixtures/reveal.mandala.json`
is this map, with three steps.
