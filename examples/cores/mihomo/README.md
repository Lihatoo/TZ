Place the downloaded Mihomo binary in this directory as `mihomo`, make it executable, and replace `REPLACE_WITH_REAL_VERSION` in `core.toml`.

Then import it with:

```bash
tz core add ./examples/cores/mihomo
```

The example manifest targets Linux x86_64. Adjust `arch` for the current Rust target architecture when needed.
