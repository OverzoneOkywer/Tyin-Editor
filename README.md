# Tyin-Editor

Tyin-Editor adalah code editor native berbasis Rust yang ringan, hemat sumber daya, dan berfokus pada optimisasi pemuatan resource (kompiler/indexer/LSP).

## Filosofi
- **Native, bukan web**: UI dibangun dengan Rust + egui/wgpu.
- **Resource-aware**: Lazy loading, on-demand LSP, virtualized UI, auto-unload tab tidak aktif.
- **Cepat untuk repo besar**: Menggunakan `ignore` + virtual list + incremental parsing dengan Tree-sitter.
- **Modern & minimal**: Layout mirip VS Code dengan sentuhan inovatif dan UI bersih.

## Struktur
Lihat `docs/ARCHITECTURE.md`.

## Build dan jalankan

Tyin-Editor mendukung build native **Linux x86_64** dan **Windows x86_64**.
GitHub Actions membangun keduanya secara otomatis pada pull request, push ke
`main`/`master`, dan tag versi `v*`. Pada tag versi, paket siap unduh
dipublikasikan ke halaman **GitHub Releases**. Build dari branch tersedia sebagai
artifact sementara di halaman run workflow.

### Instalasi dari GitHub Releases

1. Buka [Releases](https://github.com/OverzoneOkywer/Tyin-Editor/releases) dan
   unduh paket sesuai sistem operasi.
2. **Windows:** ekstrak `tyin-editor-windows-x86_64.zip`, lalu jalankan
   `tyin-app.exe`.
3. **Linux:** ekstrak `tyin-editor-linux-x86_64.tar.gz`, lalu jalankan:

   ```bash
   tar -xzf tyin-editor-linux-x86_64.tar.gz
   chmod +x tyin-app
   ./tyin-app
   ```

Paket saat ini berisi executable portable, bukan installer dengan wizard.
Linux mungkin memerlukan library runtime sistem untuk audio, windowing, dan
display. Pastikan Rust language server terpasang jika ingin menggunakan fitur
LSP.

### Linux (Ubuntu/Debian)

Pasang Rust stable melalui [rustup](https://rustup.rs/), lalu dependensi build
native berikut:

```bash
sudo apt-get update
sudo apt-get install -y \
  libasound2-dev libudev-dev libwayland-dev libx11-dev \
  libxkbcommon-dev libxkbcommon-x11-dev \
  libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev pkg-config
cargo build --locked --release -p tyin-app
./target/release/tyin-app
```

### Windows

Pasang Rust stable melalui [rustup](https://rustup.rs/) dan Visual Studio Build
Tools dengan workload **Desktop development with C++**. Jalankan di
PowerShell/Developer PowerShell:

```powershell
cargo build --locked --release -p tyin-app
.\target\release\tyin-app.exe
```

Build Windows CI menggunakan runner Windows resmi dengan toolchain MSVC.

### Cross-compile Windows dari Linux

Build Windows juga dapat dibuat dari Linux x86_64 menggunakan target GNU dan
MinGW:

```bash
sudo apt-get install -y gcc-mingw-w64-x86-64
rustup target add x86_64-pc-windows-gnu
cargo build --locked --release -p tyin-app --target x86_64-pc-windows-gnu
```

Executable hasil cross-compile berada di
`target/x86_64-pc-windows-gnu/release/tyin-app.exe`; salinan siap pakai juga
disediakan di `dist/Tyin-Editor-Windows-x86_64.exe`.

### Development

```bash
cargo run -p tyin-app
```

Binary Linux dan Windows adalah executable native, bukan installer. Linux
mungkin membutuhkan library runtime sistem untuk audio, windowing, dan display.

## Kenapa Tyin-Editor?

Tyin-Editor dirancang untuk developer yang menginginkan editor desktop native
dan alur kerja coding inti tanpa bergantung pada UI berbasis browser. Keunggulan
yang sudah tersedia:

- **Satu ruang kerja untuk coding:** Explorer, multi-tab editor, terminal, dan
  alur Git berada dalam satu aplikasi.
- **Alur Git yang bisa ditinjau:** lihat diff sebelum stage, stage/unstage file,
  dan buat commit dari panel Git.
- **Tooling yang mengikuti kebutuhan:** server LSP dijalankan dari konfigurasi,
  menyediakan diagnostics/completion/navigasi definisi, lalu dihentikan ketika
  idle.
- **Fokus pada resource:** scan workspace berjalan di background, output
  terminal dibatasi, tab yang tidak aktif dapat di-cache, dan highlighting
  dilewati untuk file di atas 256 KiB agar parsing tidak membebani UI.
- **Pilihan developer tetap sederhana:** tema gelap/terang, indentasi yang bisa
  diatur, terminal workspace, serta ikon konsisten tanpa emoji.

Tyin-Editor masih dalam pengembangan: editing file sangat besar belum
divirtualisasi, dan terminal belum mendukung aplikasi TUI yang membutuhkan
emulasi layar penuh. Klaim performa belum berdasarkan benchmark lintas mesin.

## Fitur yang sudah bisa dicoba

- Explorer workspace dan editor file dengan simpan serta undo/redo.
- Syntax highlighting Tree-sitter untuk Rust, JSON, dan TOML.
- Explorer di-refresh otomatis saat file workspace berubah, dengan urutan folder
  lebih dahulu, pemindaian di background, dan daftar file terbaru yang disimpan
  di `tyin.toml`.
- Tombol Tab memasukkan jumlah spasi dari konfigurasi `editor.tab_size`.
- Panel Git untuk meninjau diff, stage/unstage file, dan commit perubahan staged
  (`GIT` di activity bar).
- Terminal shell interaktif berbasis PTY di tab `TERMINAL` pada panel bawah.
- Output terminal mendukung warna ANSI dasar dan tombol interrupt `Ctrl+C`;
  aplikasi TUI yang membutuhkan emulasi cursor/layar penuh belum didukung.
- Diagnostics dan completion LSP jika server bahasa sudah dipasang dan
  dikonfigurasi; `Ctrl+Space` meminta completion dan `F12` menuju definisi.
- Ikon vektor pada activity bar serta ikon aplikasi Tyin.
- Tema gelap/terang dapat diganti dari command palette dan disimpan ke konfigurasi.
- Konfirmasi sebelum membuang perubahan saat menutup tab, mengganti workspace,
  atau keluar dari aplikasi.

Contoh konfigurasi server Rust di `tyin.toml`:

```toml
[lsp.servers.rust]
command = "rust-analyzer"
args = []
```

Server harus tersedia di `PATH`. Dukungan LSP mencakup initialize, lifecycle
dokumen, diagnostics, completion, navigasi definisi, dan penghentian server
yang idle. File besar di atas 256 KiB tetap dapat dibuka dan diedit, tetapi
syntax highlighting dinonaktifkan untuk menghindari parsing sinkron yang berat.
Buffer teks masih dimuat penuh ke memori dan pengeditan file sangat besar belum
divirtualisasi.

## Target MVP
- [x] Workspace scaffold
- [x] VSCode-like layout (Activity Bar, Sidebar, Editor, Panel, Status Bar)
- [x] File tree (virtualized)
- [x] Multi-tab editor dengan ropey buffer
- [x] Syntax highlighting via Tree-sitter
- [x] File watcher + pembaruan Explorer
- [x] Resource monitor panel
- [x] Lazy/on-demand LSP pool (server command dikonfigurasi pengguna)
- [x] Theme system (dark/light)

## Lisensi
MIT OR Apache-2.0
