# TODO

## Outstanding

- **Filesystem on WASM.** Native open/save dialogs are wired through the
  dispatch funnel (`Action::OpenDocument`, `SaveDocumentAs`, `NewDocumentAt`)
  but the WASM side has no equivalent — these variants are NativeOnly today.
  Wiring needs File System Access API for save and a download fallback for
  browsers that don't expose it.

For shipped work see `git log`.
