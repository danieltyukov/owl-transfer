## What this changes

<!-- What the change is and why it is right. One or two paragraphs. -->

## How it was checked

<!--
Which of these you ran, and anything you tried by hand. CI runs the first
group on every push, so a note here is about what CI cannot see: two real
devices, a phone, a large file, a network that drops.

  npm run typecheck
  npm test
  npm run test:e2e
  cargo test -p owl-core
  cargo clippy -p owl-core --all-targets -- -D warnings
  cargo fmt --check
-->

## Notes

<!--
Anything a reviewer would otherwise have to work out: a wire format change and
whether it stays compatible with a peer running the previous release, a change
to what is written in the data directory, a new permission, a new dependency
and why it earns its place.

Two rules the review will check for, because both are easy to break by
accident: `crates/core` does not mention Tauri or any UI, and nothing under
`app/src` imports a Tauri module except `app/src/backend/tauri.ts`.

No emojis, and no em or en dashes, in code, comments, documentation or the
commit message.
-->
