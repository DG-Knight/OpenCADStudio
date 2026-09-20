# Driving the native builds

Every native build exposes the same control core through two entry points: the MCP
server described in the [README](README.md) (`OpenCADStudio --mcp`), and a headless
JSON-line channel meant for scripts and CI (`OpenCADStudio --serve`). The operations,
request envelope and replies are the same; this page covers what is specific to the
native side. The browser entry is described in [web.md](web.md).

## Choosing a channel

| | `--mcp` | `--serve` |
|---|---|---|
| Transport | MCP over stdio | one JSON request per line on stdin, one reply per line on stdout |
| Editor | starts the desktop editor and exposes the live document | no window; works on its own document |
| Needs a display | yes — on a headless machine `xvfb-run` works, on software rendering the viewport may show the GPU warning dialog | no |
| `measure`, `ocs_capture`, `set_properties`, interactive steps | available | `measure` and `capture` are not; the remaining operations are |
| Good for | an MCP client driving a real editor | batch jobs, CI, agents without a display |

`OpenCADStudio --export IN OUT` converts a drawing in one shot, without any session. It
accepts `.dwg` and `.dxf` targets only.

## Requirements

The process needs a writable `HOME` (or `XDG_CONFIG_HOME`) — without one, `ocs_sessions`
answers `No user configuration directory` and nothing else works.

## Startup dialogs

A fresh profile opens two dialogs before any drawing exists, and both of them block
document creation:

| Step | `ocs_sessions` reports |
|---|---|
| right after launch | `modal: "AssocPrompt"` |
| after `close_modal` | `modal: "DonationPrompt"` |
| after a second `close_modal` | `modal: null` |

While a dialog is open, `{"op":"new"}` answers `ok` but no tab is created, so there is
no `document_id` to work with. Close the dialogs first:

```python
while (info := sessions())["modal"]:
    execute({"op": "action", "name": "close_modal", "document_id": info["document_id"]})
```

The donation prompt appears once per application version per profile, so an automated
profile meets it once per version rather than on every run. See
[web.md](web.md) for the same pattern on the browser side.

## Targeting a document

Reads and mutations target the active tab. `new` creates a document but does not move
the active tab, so pass the `document_id` from the reply explicitly — without it, reads
answer empty and it looks like nothing was created. `activate` switches the active tab.
A client connection to `--mcp` starts a new editor instance, so session state does not
carry over between connections.

## Feeding a whole command line

`run` takes one complete command line, including the answers to every prompt, and then
terminates the command as if Enter were pressed. A prompt cannot be continued on a
following line — each line is parsed as a fresh command line, so a command left waiting
is replaced by the next one (send `{"op":"cancel"}` if you want to drop it explicitly).

`waiting_input` means input is still required — not that nothing happened. `SPLINE` and
`ARC`, for example, keep accepting further input even after the entity exists, and
`zoom_extents` reports it while the view has already changed. Send `{"op":"cancel"}` to
close the command when the line is done with it.

Two reply fields describe what happened to the line:

| Field | Meaning |
|---|---|
| `unconsumed` | Tokens that no prompt ever asked for, in order. `CIRCLE 0,0 5 9` reports `["9"]`, because the radius step ends the command; a fully consumed line reports `[]`. |
| `blocked_by` | What *this line* left open: `"command"`, `"text_editor"`, `"mtext_editor"`, `"modal:<Kind>"`, or `null`. Set whenever `status` is `waiting_input`. |

`TEXT 0,0 5 0 hello world` therefore creates one `Text` entity that holds
`hello world`, reports `completed` with `added: 1`, and leaves nothing unconsumed. The
content step runs in the in-place editor, which the feeder commits and then closes so
that the command does not stay armed for a second line.

## Interactive steps

For commands whose answers cannot be written on one line, use `start` and then `input`.
`start` replies with `state.command`, whose `accepts`, `options`, `prompt` and
`input_example` say what the current step wants:

| Step | Request | Note |
|---|---|---|
| a point | `{"op":"input","kind":"point","point":[x,y,0],"space":"wcs"}` | the point is an array; separate `x`/`y` fields are rejected |
| a typed value (height, rotation) | `{"op":"input","kind":"text","text":"5"}` | digits go through `text`, not `token`; `{"op":"input","kind":"enter"}` accepts the default |
| a keyword (`J`, `ST`, `C`) | `{"op":"input","kind":"token","text":"C"}` | the payload field is named `text` |
| the text of `TEXT`/`MTEXT` | `{"op":"action","name":"text_input","value":"hello"}` then `{"op":"action","name":"text_commit"}` | the command has already ended at that point and the editor owns the input |

## Native-only operations

| Operation | Device |
|---|---|
| `measure` | `ocs_read` with `op: "measure"` and `parameters.handles` returns kernel-computed length, area, bounds and mass properties. |
| `ocs_capture` | Returns the viewport or the whole window. Either an inline base64 image, or a PNG written under the temporary directory with the reply carrying its `path`. |
| `set_properties` | Mutates record properties. It requires `collection`, and colours are the serialised enum — `{"Index": 1}` or `"ByLayer"`; a colour name such as `"Red"` is rejected with `invalid_value`. |

## Known gaps

On the `--serve` path, `MTEXT`, `HATCH` and `POLYGON` report `completed` with
`added: 0`: they need interactive surfaces (a pattern choice, a boundary selection) that
a single command line has no tokens to fill. `TEXT` no longer belongs to that list.
