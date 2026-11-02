# lightningcss migration, October 2026

## What was migrated

| Upstream parcel-bundler/lightningcss PR | Target rstackjs/lightningcss PR | Change |
| --- | --- | --- |
| #1289 (already ported) | #1 | Custom source map implementations |
| #1333 | #3 | Factor common custom properties in adjacent rules |
| #1334 | #4 | Deduplicate unparsed text property prefixes |
| #1335 | #5 | Minify zero-valued translate3d in one pass |
| #1336 | #6 | Preserve default border-image shorthand values |
| #1338 | #7 | Choose shorter quotes for minified URLs |
| #1339 | #8 | Deduplicate mask-image prefixes |

The six ports initially targeted `master` at `5aec9d438d49245f17070186841560b518d28819`. Final merged `master` was `a3cf0d9b0e12d566501819c768e037585747894d`. These SHAs describe this run; discover fresh inputs for the next migration.

Original upstream commits were retained by creating merge commits with two parents: the upstream PR head and the target base. Their trees matched the separately tested port trees exactly. The reusable `preserve` command translates that Python `git commit-tree` helper into zx. `discover` replaces ad hoc PR metadata collection; `prepare` replaces the temporary shared clone and patch-application loop with a direct history-preserving merge; `verify` replaces the ancestry/tree comparison script.

The original patch loop appended new tests from pure insertion hunks when contexts differed. Do not generalize that into an automatic patch strategy: adjacent upstream and target test insertions need inspection, and arbitrary hunks can include deletions or multiple insertion sites. Direct merges preserve Git's conflict evidence.

## Adaptations that mattered

**Default branch and fork state.** The repository calls its default branch `master`, not `main`. Keep `rspack_lightningcss`, version `0.1.0`, fork-specific dependencies, release workflows, and existing upstream regression tests. Never replace complete target files with source files just to make a patch apply.

**Border image outset.** Upstream #1336 compared `Rect<LengthOrNumber>` values. The target had moved to `Rect<NonNegative<LengthOrNumber>>`. Keep `NonNegative(LengthOrNumber::Number(0.0))` in the default comparison.

**Printer imports.** After #1, #3 conflicted around `src/rules/mod.rs` imports. Keep the new extraction's `Printer`, `PrinterOptions`, `CustomPropertyName`, and `Property` imports alongside the trait-based serializers.

**Printer option access.** In `src/properties/transform.rs`, combine #5's removal of `!x.is_zero()` with #1's accessor change:

```rust
if dest.options().minify && y.is_zero() && z.is_zero() {
```

Keeping only either side silently loses the optimization or fails to compile.

**Printer error conversion.** In `src/properties/border_image.rs`, the shared serializer now takes `dest: &mut PrinterT` with `PrinterT: PrinterTrait`. Its `write_str` returns a formatting error. The initial shorthand case must convert that error:

```rust
dest.write_str("none")?;
return Ok(());
```

Returning `dest.write_str("none")` directly no longer matches `Result<(), PrinterError>`.

**Source map calls in clean merges.** #7 had no textual conflict, but its new tests still used the old stylesheet API. Adapt `test_url_quoting_dependencies` and `test_url_quoting_source_map` in `src/lib.rs` to pass the source map separately:

```rust
stylesheet.to_css(options, None::<&mut ()>)?;
stylesheet.to_css(options, Some(&mut source_map))?;
```

Remove `source_map` from `PrinterOptions`. Account for the actual enabled feature configuration: this separate argument is behind `custom_sourcemap`; `sourcemap` enables it in the tested configuration. Existing helper tests already followed the new API.

## Validation and order

Every source PR's CI initially passed, but merging #1 changed APIs for later ports. The final order was #1, #3, #4, #5, #6, #7, #8. #3, #5, and #6 needed conflict resolutions; #7 needed compilation adaptations despite a clean textual merge. Revalidate each changed combined tree and the final default branch.

`bundler::tests::test_bundle` sometimes failed on `@media screen, print` versus expected `@media print, screen`, independently of the property fixes. Inspect the assertion and confirm it is this known ordering issue. A single complete rerun passed in the affected runs; document both attempts. Do not assume every bundler failure has this cause, change expected output, retry indefinitely, or claim an unconditionally clean first run.

The target disallowed merge commits, so the PRs were merged using squash after explaining the policy limitation. That retained originals in PR branches, not as ancestors on the default branch. Do not claim original SHAs survived on `master`.
