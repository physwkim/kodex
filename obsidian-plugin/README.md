# Engram Obsidian Plugin

Query and explore engram knowledge graphs directly inside Obsidian.

## Features

- **Query** (`Ctrl+P` → "Engram: Query") — search the knowledge graph with natural language
- **Explain** (`Ctrl+P` → "Engram: Explain current note") — show connections for the active note
- **Path** (`Ctrl+P` → "Engram: Find path") — find shortest path between two concepts
- **God Nodes** (`Ctrl+P` → "Engram: Show god nodes") — list most-connected entities
- **Rebuild** (`Ctrl+P` → "Engram: Rebuild") — re-extract and update the graph

## Requirements

- `engram` binary in PATH (or configured in settings)
- A `graph.json` file in the vault (run `engram run .` first)

## Setup

1. Build engram: `cargo build --release`
2. Run on your project: `engram run ./my-project`
3. Open `my-project/engram-out/` as Obsidian vault
4. Copy this plugin to `.obsidian/plugins/engram/`
5. Enable in Obsidian settings → Community plugins

## Settings

| Setting | Description | Default |
|---------|-------------|---------|
| engram binary path | Path to engram executable | `engram` |
| graph.json path | Path to graph.json relative to vault | `graph.json` |

## Live Sync

Use `engram watch` with `--vault` to keep the vault updated as you code:

```bash
engram watch ./my-project --vault ~/obsidian-vault/my-project
```

Edits in Obsidian (adding/removing `[[wikilinks]]`) are synced back to `graph.json` automatically.
