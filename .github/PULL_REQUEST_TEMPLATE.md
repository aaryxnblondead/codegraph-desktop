## What and why

<!-- Two or three sentences. Link the issue this closes. -->

Closes #

## How

<!-- The approach, and anything a reviewer would otherwise have to guess. -->

## Verification

<!-- Paste the commands you ran and their result. CI runs the same set. -->

```
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo build --release
```

## Checklist

- [ ] A test covers the change (or the change is docs-only)
- [ ] No `unwrap()` added outside tests
- [ ] User-facing text is plain and short
- [ ] Analysis stays in the headless modules; egui only in `src/ui.rs` / `src/main.rs`
- [ ] Nothing writes inside the indexed project
- [ ] `CHANGELOG.md` / docs updated if behaviour or output changed

## Notes for reviewers

<!-- Tradeoffs, follow-ups, screenshots, anything worth a second look. -->
