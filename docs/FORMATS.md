# ROM Display Formats

Unlike prior art, ROM features several different display and legend formats as
opposed to NOM's immutable design. This allows for the freedom to mix and match
different component styles in the build graph.

## Display Formats

ROM supports three display formats controlled by the `--format` flag:

1. Tree Format (Default)
2. Plain Format
3. Dashboard Format

### 1. Tree Format (Default)

The tree format shows a hierarchical dependency graph with build progress.

**Usage:**

```bash
rom --format tree build nixpkgs#hello
# or simply (tree is default)
rom build nixpkgs#hello
```

### Examples

**Tree Format**:

```plaintext
┏━ Dependency Graph:
┃ ⏵ hello-2.12.2 (buildPhase) ⏱ 5s
┣━━━ Builds
┗━ ∑ ⏵ 1 │ ✔ 0 │ ✗ 0 │ ⏸ 4 │ ⏱ 5s
```

**Plain Format**:

```plaintext
━ ⏱ ⏸ 4 planned ↓ 2 downloading ↑ 1 uploading 5.7s
  ↓ breakpad-2024.02.16 1.2 MB/5.0 MB (24%)
  ↓ spirv-tools-1.4.321.0 0 B
  ↑ gcc-13.2.0 250 KB
  ⏵ hello-2.12.2 5s
```

**Dashboard Format**:

```plaintext
BUILD GRAPH: hello-2.12.2
────────────────────────────────────────────
Host        │ localhost
Status      │ ⏵ building
Duration    │ 8.1s
────────────────────────────────────────────
Summary     │ jobs=1  ok=1  failed=0  total=8.1s
```

## Legend Styles

Legend styles control how the build statistics are displayed at the bottom of
the screen. At this moment they only affect the **tree format**.

1. Table Style
2. Compact Style
3. Verbose Style

### Examples

**Table**:

```plaintext
┏━ Dependency Graph:
┃ ⏵ hello-2.12.2 (buildPhase) ⏱ 5s
┣━━━ Builds
┗━ ∑ ⏵ 1 │ ✔ 0 │ ✗ 0 │ ⏸ 4 │ ⏱ 5s
```

**Compact**:

```plaintext
┏━ Dependency Graph:
┃ ⏵ hello-2.12.2 (buildPhase) ⏱ 5s
┗━ ⏵ 1 │ ✔ 0 │ ✗ 0 │ ⏸ 4 │ ⏱ 5s
```

**Verbose**:

```plaintext
┏━ Dependency Graph:
┃ ⏵ hello-2.12.2 (buildPhase) ⏱ 5s
┣━━━ Build Summary:
┗━ ⏵ 1 running │ ✔ 0 completed │ ✗ 0 failed │ ⏸ 4 planned │ ⏱ 5s
```

## Icon Legend

All formats use consistent icons:

| Icon | Meaning           | Color  |
| ---- | ----------------- | ------ |
| ⏵    | Building/Running  | Yellow |
| ✔    | Completed/Success | Green  |
| ✗    | Failed/Error      | Red    |
| ⏸    | Planned/Waiting   | Grey   |
| ⏱    | Time/Duration     | Grey   |
| ↓    | Downloading       | Yellow |
| ↑    | Uploading         | Yellow |
