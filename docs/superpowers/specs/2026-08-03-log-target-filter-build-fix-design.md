# Log Target Filter Build Fix

## Problem

The log-center change configures the folder target with
`Target::level(log::LevelFilter::Error)`. `tauri-plugin-log` 2.8.0 does not
provide a `level` method on `Target`, so the Rust backend cannot compile.

## Design

Use the plugin's target-specific `filter` API on the existing folder target.
The predicate accepts only records whose level is `log::Level::Error`.

This preserves the current behavior and configuration:

- stdout and the diagnostic database continue to receive records allowed by
  the builder-level runtime filter;
- `cc-switch.log` contains only error records;
- the folder target continues to use the plugin's 4 MiB rotating writer,
  `KeepSome(4)` retention, append behavior, and local timestamps.

No custom file writer is introduced. The diagnostic log storage and GitHub
Actions dependency setup remain unchanged.

## Error Handling

The existing plugin initialization and log-directory error handling remain in
place. The filter is a pure predicate and introduces no new failure path.

## Verification

1. Run Rust formatting checks.
2. Compile or check the Rust crate against the locked dependencies.
3. Run the focused Rust tests, or the full Rust test suite when available.
4. Confirm the source no longer calls `Target::level`.
