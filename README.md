# Codex_PG

Generation Playground.

## TNS MVP Mode

This repo now includes a **TNS MVP external brain package demo**.

Run:

```bash
cargo run -- tns-mvp
```

It will:
- Load `brain_packages/example.pattern_brain.v0` from disk.
- Verify manifest + content hashes + probe receipt presence.
- Bind through a read-only adapter.
- Traverse declared regions.
- Emit a traversal receipt proving brain immutability and proposal-only output status.
