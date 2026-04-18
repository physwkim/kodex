# Graphify Obsidian Plugin

Query and explore graphify knowledge graphs directly inside Obsidian.

## Features

- **Query** (`Ctrl+P` → "Graphify: Query") — search the knowledge graph with natural language
- **Explain** (`Ctrl+P` → "Graphify: Explain current note") — show connections for the active note
- **Path** (`Ctrl+P` → "Graphify: Find path") — find shortest path between two concepts
- **God Nodes** (`Ctrl+P` → "Graphify: Show god nodes") — list most-connected entities
- **Rebuild** (`Ctrl+P` → "Graphify: Rebuild") — re-extract and update the graph

## Requirements

- `graphify` binary in PATH (or configured in settings)
- A `graph.json` file in the vault (run `graphify run .` first)

## Setup

1. Build graphify: `cargo build --release`
2. Run on your project: `graphify run ./my-project`
3. Open `my-project/graphify-out/` as Obsidian vault
4. Copy this plugin to `.obsidian/plugins/graphify/`
5. Enable in Obsidian settings → Community plugins

## Settings

| Setting | Description | Default |
|---------|-------------|---------|
| graphify binary path | Path to graphify executable | `graphify` |
| graph.json path | Path to graph.json relative to vault | `graph.json` |

## Live Sync

Use `graphify watch` with `--vault` to keep the vault updated as you code:

```bash
graphify watch ./my-project --vault ~/obsidian-vault/my-project
```

Edits in Obsidian (adding/removing `[[wikilinks]]`) are synced back to `graph.json` automatically.
