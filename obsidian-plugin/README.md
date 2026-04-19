# Kodex Obsidian Plugin

Query and explore kodex knowledge graphs directly inside Obsidian.

## Features

- **Query** (`Ctrl+P` → "Kodex: Query") — search the knowledge graph with natural language
- **Explain** (`Ctrl+P` → "Kodex: Explain current note") — show connections for the active note
- **Path** (`Ctrl+P` → "Kodex: Find path") — find shortest path between two concepts
- **God Nodes** (`Ctrl+P` → "Kodex: Show god nodes") — list most-connected entities
- **Rebuild** (`Ctrl+P` → "Kodex: Rebuild") — re-extract and update the graph

## Requirements

- `kodex` binary in PATH (or configured in settings)
- A `graph.json` file in the vault (run `kodex run .` first)

## Setup

1. Build kodex: `cargo build --release`
2. Run on your project: `kodex run ./my-project`
3. Open `my-project/kodex-out/` as Obsidian vault
4. Copy this plugin to `.obsidian/plugins/kodex/`
5. Enable in Obsidian settings → Community plugins

## Settings

| Setting | Description | Default |
|---------|-------------|---------|
| kodex binary path | Path to kodex executable | `kodex` |
| graph.json path | Path to graph.json relative to vault | `graph.json` |

## Live Sync

Use `kodex watch` with `--vault` to keep the vault updated as you code:

```bash
kodex watch ./my-project --vault ~/obsidian-vault/my-project
```

Edits in Obsidian (adding/removing `[[wikilinks]]`) are synced back to `graph.json` automatically.
