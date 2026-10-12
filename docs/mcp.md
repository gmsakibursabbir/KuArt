# MCP server (AI agents)

KuArt ships a [Model Context Protocol](https://modelcontextprotocol.io) server so AI agents
(Claude Code, Claude Desktop, any MCP client) can create designs, edit them, look at them and turn
them into code.

```
design-editor-cli mcp
```

Transport: stdio, newline-delimited JSON-RPC 2.0. It runs headless (CPU renderer, no window, no
GPU) and never goes online. One document is open per session; every edit is undoable.

## Setup

Claude Code (project `.mcp.json` or `claude mcp add`):

```bash
claude mcp add kuart -- design-editor-cli mcp
```

Claude Desktop (`claude_desktop_config.json`):

```json
{ "mcpServers": { "kuart": { "command": "C:\Program Files\KuArt\design-editor-cli.exe", "args": ["mcp"] } } }
```

## Tools

| Tool | Purpose |
|---|---|
| `open_document` | Open .krt, Figma files, .xd, .psd, .svg, images |
| `new_document` | New document; `width`/`height` add an artboard |
| `get_tree` | Layer tree with ids, kinds, names, positions, sizes |
| `get_node` | All properties of one node (JSON) |
| `create_node` | frame, rectangle, ellipse, polygon, star, line, text with position, size, fill, radius, text and font |
| `update_node` | Change any of those properties |
| `delete_node`, `undo`, `redo` | Editing |
| `render_preview` | PNG image of a node, plus missing-font names |
| `generate_code` | html, css, tailwind, react, vue or tokens (see `dev-mode.md`) |
| `export`, `export_svg_code` | PNG/SVG files or SVG markup |
| `save_document` | Save as .krt (open it in KuArt) |

Node ids are the numbers shown by `get_tree`. Coordinates are pixels relative to the parent.
Colours are `#RGB`, `#RRGGBB` or `#RRGGBBAA`.

The same code generator is available without an agent:

```bash
design-editor-cli code design.krt react ProductCard.jsx
```
