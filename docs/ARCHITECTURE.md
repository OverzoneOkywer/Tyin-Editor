# Arsitektur Tyin-Editor

## Prinsip
- **Lazy by default**: Hanya muat apa yang dibutuhkan saat ini.
- **Event-driven**: Komunikasi antar komponen lewat event bus ringan.
- **Detachable resources**: LSP/watcher/indexer bisa dimatikan saat idle.
- **Cache-aware**: AST/highlight result dicache per (file_id, version).

## Crates
| Crate | Tugas |
|---|---|
| `tyin-app` | Entry point, bootstrap tracing/config, jalankan `eframe`. |
| `tyin-core` | Config, event bus, `ResourceManager`, error/types umum. |
| `tyin-fs` | VFS, `.gitignore` handling, `notify` watcher. |
| `tyin-text` | Rope buffer, cursor/selection, undo/redo, edit ops. |
| `tyin-syntax` | Tree-sitter grammar loader, highlight, incremental parse. |
| `tyin-lsp` | LSP client pool, lazy spawn, idle timeout, throttle. |
| `tyin-ui` | Layout, panels, theme, command palette, resource monitor. |

## Alur
1. `tyin-app` load `tyin.toml` -> init `ResourceManager`.
2. UI render layout. User buka folder -> `tyin-fs` scan (filtered).
3. Buka file -> `tyin-text` buat buffer (ropey). Jika didukung -> `tyin-syntax` parse+highlight on-demand.
4. Fitur LSP baru dipanggil saat dibutuhkan -> `tyin-lsp` spawn jika belum ada, track usage.
5. `ResourceManager` monitor usage & bisa unload background state.

## Resource-Aware Details
- **Tab lifecycle**: `active` / `cached` / `unloaded`. Unload setelah `idle_secs`.
- **Virtualized lists**: File tree & search results render visible range.
- **Coalescing**: Debounce file writes & highlights.
- **On-demand LSP**: Pool per workspace+language, shutdown on idle.
