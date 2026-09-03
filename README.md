<div align="center">

# diagram-ir

### Deterministic draw.io and Mermaid extraction to a normalised IR

[![Licence](https://img.shields.io/badge/Licence-MIT%20OR%20Apache--2.0-blue?style=flat-square)](LICENSE-MIT)
[![crates.io](https://img.shields.io/crates/v/diagram-ir?style=flat-square)](https://crates.io/crates/diagram-ir)
[![CI](https://img.shields.io/github/actions/workflow/status/DreamLab-AI/diagram-ir/ci.yml?style=flat-square)](https://github.com/DreamLab-AI/diagram-ir/actions)

</div>

---

## What is diagram-ir?

diagram-ir parses draw.io XML (including the double-compressed format draw.io
actually writes) and Mermaid text into a normalised intermediate representation:
typed nodes, typed edges, labels, positions and structural signals, as a single
Rust struct that both input formats share. A Markdown digest and a JSON IR are
the two output surfaces.

diagram-ir exists because an LLM agent that can read a diagram file cannot
reliably read the format the file is stored in. draw.io XML is deflate-inside-
URL-encode-inside-base64, nested inside an `mxGraphModel` whose style strings
encode layout, and Mermaid syntax carries enough variation across diagram kinds
that a bare regex pass loses structure. diagram-ir handles the decoding,
flattening, and structural analysis, then hands the agent a table it can reason
over without touching XML or guessing at compression.

A trust boundary is the defining constraint. Every entry point parses bounded
text or bytes. Nothing in this crate evaluates, renders, fetches or executes its
input. No network, no subprocesses, no DTD or entity expansion, and hard caps on
input size, decompressed size, node count and edge count.

Pure Rust with seven dependencies (clap, serde, serde_json, regex,
quick-xml, flate2, base64), no C code and no build script. 100 tests cover the
draw.io decoder, all four Mermaid diagram kinds, the golden-parity contract
between formats, and the self-check rules.

## Where diagram-ir sits in the ecosystem

diagram-ir is one component of
**[VisionFlow](https://github.com/DreamLab-AI/VisionFlow)**, extracted from
agentbox into its own repository. Inside agentbox it backs the `diagram-design`
skill, which uses the IR to let an agent understand, critique and redraw a
diagram without ever evaluating the source format.

| Sibling | Relationship |
|:--------|:-------------|
| [agentbox](https://github.com/DreamLab-AI/agentbox) | Consumes the binaries via a pinned Nix derivation and the `diagram-design` skill |
| [VisionFlow](https://github.com/DreamLab-AI/VisionFlow) | Ecosystem canon |

## Architecture

### Modules

| Module | Role |
|:-------|:-----|
| `drawio` | Decode (deflate, URL-decode, base64), flatten `mxGraphModel` to absolute-positioned nodes and edges, extract style properties |
| `mermaid` | Parse bounded Mermaid text: flowchart, sequenceDiagram, stateDiagram-v2, erDiagram. Lexer, per-kind parser, label extraction, shape mapping |
| `selfcheck` | The accessible-diagram contract: SVG structure rules, single-file safety, motion-controller parity. Packaged as a library and as the `diagram-self-check` binary |
| `markdown` | Locate fenced Mermaid blocks inside a Markdown file |
| `pyfmt` | Python format-string interpolation for template variables |
| `xmldom` | Minimal DOM over quick-xml events, with attribute and depth limits |
| `entities` | HTML entity resolution without pulling in a full HTML parser |

### Trust boundary

| Limit | Default |
|:------|:--------|
| Source input | 4 MiB |
| Max nodes | 2,000 |
| Max edges | 5,000 |
| XML attributes per element | Bounded by xmldom |
| XML nesting depth | Bounded by xmldom |

All limits are hard: exceeding one is a `Fail`, not a silent truncation.

### Binaries

**`drawio-extract`** extracts from `.drawio`, `.xml`, `.drawio.png` or
`.drawio.svg` files.

```
drawio-extract [OPTIONS] <FILE>
  --page <PAGE>       page index, page name, or 'all' (default: first page)
  --json              emit the full IR as JSON
  --max-rows <N>      rows per table in the Markdown digest (default: 40)
  --out <PATH>        write to this path instead of stdout
```

**`mermaid-extract`** extracts from `.mmd`, `.mermaid` or Markdown with mermaid
fences.

```
mermaid-extract [OPTIONS] <FILE>
  --diagram <INDEX>   diagram index or 'all' (default: first diagram)
  --json              emit the full IR as JSON
  --max-rows <N>      rows per table in the Markdown digest (default: 40)
  --out <PATH>        write to this path instead of stdout
```

**`diagram-self-check`** validates generated diagram HTML files against the
accessible-diagram contract.

```
diagram-self-check [OPTIONS] <FILES>...
  --motion-template <PATH>  path to the canonical template-motion.html
```

The canonical motion controller is resolved in this order: `--motion-template`,
`$DIAGRAM_DESIGN_SKILL_DIR/assets/template-motion.html`, the installed skill at
`/opt/agentbox/skills/diagram-design/assets/`, `./skills/diagram-design/assets/`,
and finally the copy compiled into the binary from `assets/template-motion.html`.
A standalone install therefore needs no skill checkout; an installed skill wins
when present so a repository can pin its own controller.

Exit codes: **0** every file passes, **1** at least one file fails the contract (each failure is listed), **2** tool error.

## Quickstart

```sh
# From source (needs Rust 1.85+):
cargo install diagram-ir

# Extract a draw.io file to Markdown:
drawio-extract architecture.drawio

# Extract to JSON IR:
drawio-extract --json architecture.drawio > ir.json

# Extract all Mermaid diagrams from a Markdown file:
mermaid-extract --diagram all design.md

# Self-check a generated diagram:
diagram-self-check output.html
```

## Measured

100 tests across seven test files:

- draw.io decoding: compressed, double-compressed, multi-page, style extraction
- Mermaid parsing: flowchart, sequenceDiagram, stateDiagram-v2, erDiagram
- Golden parity: the same logical diagram in both formats produces structurally
  equivalent IR
- Self-check: the accessible-diagram contract rules
- Lexer and grammar coverage for each Mermaid kind

## What it does not do

- **Render.** The IR is data, not pixels. Rendering is the caller's job.
- **Evaluate.** No JavaScript, no Mermaid directives, no click handlers, no URLs
  are followed.
- **Write draw.io or Mermaid.** The tool reads; it does not round-trip.

## Status

Stable for the draw.io and Mermaid subset agentbox uses. Mermaid coverage is
four diagram kinds (flowchart, sequence, state, ER); the remaining kinds
(class, Gantt, pie, mindmap, etc.) are not yet parsed. draw.io coverage is the
full `mxGraphModel` format including compressed pages.

## Releasing

See [RELEASE.md](RELEASE.md).

## Licence

**MIT OR Apache-2.0**, at your option.

Copyright (c) 2026 DreamLab AI Consulting Ltd and contributors.
