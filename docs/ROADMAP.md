# Roadmap Tyin-Editor

Dokumen ini menjadi urutan kerja produk. Prioritasnya adalah editor yang aman
untuk digunakan sehari-hari, lalu navigasi dan tooling, kemudian ekosistem
extension. Rencana extension sengaja belum menetapkan desain teknis sebelum
kebutuhannya dibahas lebih lanjut.

## Status saat ini

- [x] Workspace Rust dan pemisahan crate dasar
- [x] UI native egui/wgpu dengan layout awal
- [x] Explorer dengan expand/collapse folder
- [x] Buffer Rope dan undo/redo dasar, dengan batas 100 langkah
- [x] Buka dan simpan file dasar
- [ ] Fitur-fitur berikut masih perlu penguatan sebelum dianggap siap pakai

## Tahap 1 — Keandalan editor dan data

- [x] Sinkronkan perubahan UI dengan buffer Rope dan gunakan buffer untuk dirty
      state serta penyimpanan
- [x] Beri peringatan sebelum menutup tab yang memiliki perubahan belum disimpan
- [x] Tampilkan error buka file/workspace, simpan, dan reload dengan jelas
- [x] Pertahankan metadata path buffer setelah Save As
- [x] Uji undo/redo dan dirty state pada buffer
- [x] Tambahkan tes save/reload file
- [ ] Tambahkan tes alur konfirmasi penutupan tab
- [x] Beri peringatan sebelum mengganti workspace atau keluar dengan tab dirty

**Selesai jika:** perubahan pengguna tidak hilang diam-diam dan kegagalan file
ditampilkan secara jelas.

## Tahap 2 — Navigasi dan file system

- [x] Jadikan tree Explorer deterministik: folder dikelompokkan sebelum file
      dengan pengurutan nama konsisten
- [x] Pindahkan pemindaian workspace ke worker agar UI tidak menunggu scan
- [x] Perbarui Explorer saat watcher menerima perubahan file/direktori
- [x] Tambahkan buka file dan buka workspace melalui command palette
- [x] Tangani symlink, file yang hilang, serta path di luar workspace dengan aman
- [x] Tambahkan daftar file terbaru

**Selesai jika:** navigasi workspace tetap akurat saat file berubah di luar editor.

## Tahap 3 — Pengalaman editor dan syntax

- [x] Selaraskan nomor baris dengan area teks saat scroll dan resize
- [x] Terapkan konfigurasi indentasi/tab size
- [x] Implementasikan syntax highlighting Tree-sitter untuk Rust, JSON, dan TOML
- [x] Lewati parsing syntax sinkron untuk file di atas 256 KiB
- [x] Cache hasil syntax per buffer dan perbarui saat teks berubah
- [ ] Pastikan editing file besar tidak memblokir UI

**Selesai jika:** fitur editor dasar bekerja konsisten dan parsing tidak membuat UI
tersendat.

## Tahap 4 — Identitas visual dan sistem ikon

- [x] Tetapkan nama, palet aksen biru, dan gaya visual Tyin-Editor
- [x] Buat mark Tyin serta ikon aplikasi awal
- [x] Gunakan ikon vektor konsisten di activity bar
- [x] Jangan gunakan emoji sebagai ikon; sediakan tooltip untuk tombol ikon
- [x] Sediakan tema gelap dan terang dengan warna aksen dan teks seleksi kontras

**Selesai jika:** aksi UI dikenali melalui ikon yang konsisten, jelas, dan tetap
aksesibel.

## Tahap 5 — Integrasi Git

- [x] Deteksi repository dan tampilkan branch serta status perubahan
- [x] Tampilkan dekorasi file berubah/tidak terlacak di Explorer
- [x] Sediakan tampilan diff sebelum perubahan di-stage atau di-commit
- [x] Tambahkan aksi Git dasar secara bertahap: stage, unstage, commit, dan
      refresh status
- [x] Tampilkan error Git dan konflik dengan jelas; jangan mengubah file secara
      diam-diam

**Selesai jika:** pengguna dapat meninjau perubahan dan menjalankan alur Git dasar
dari UI dengan hasil yang dapat dipahami.

## Tahap 6 — Terminal terintegrasi

- [x] Tambahkan terminal ke panel bawah dengan tab Output, Problems, dan Terminal
- [x] Gunakan PTY agar shell interaktif berjalan semestinya
- [x] Mulai di direktori workspace aktif dan dukung resize terminal
- [x] Hentikan proses terminal saat aplikasi keluar atau workspace berganti
- [x] Batasi buffer output dan proses repaint agar penggunaan resource terkendali
- [x] Tampilkan error saat shell gagal dimulai atau proses berakhir

**Selesai jika:** shell interaktif dapat digunakan dalam workspace tanpa membuat
UI macet atau membiarkan proses yatim.

Catatan: terminal merender warna ANSI dasar dan dapat mengirim Ctrl+C, tetapi
belum mengemulasi kontrol cursor/layar penuh untuk aplikasi TUI.

## Tahap 7 — LSP dan resource management

- [x] Jalankan server bahasa berdasarkan konfigurasi workspace/bahasa
- [x] Implementasikan initialize/shutdown, open/change dokumen, dan diagnostics
- [x] Terapkan idle shutdown yang benar-benar menghentikan proses LSP
- [x] Tampilkan diagnostics pada panel Problems
- [x] Implementasikan completion dan navigasi definisi simbol
- [ ] Ukur penggunaan resource saat membuka workspace besar

**Selesai jika:** LSP hanya berjalan saat dibutuhkan dan lifecycle prosesnya
terpantau.

## Tahap 8 — Perencanaan extension (dibahas lebih lanjut)

- [ ] Sepakati tujuan extension: konfigurasi bahasa, tema, command, atau plugin
- [ ] Tentukan model distribusi, kompatibilitas, izin, dan batas keamanan
- [ ] Bandingkan extension bawaan, konfigurasi eksternal, WASM, dan proses terpisah
- [ ] Rancang API/versioning setelah kebutuhan dan batas kepercayaan disepakati
- [ ] Implementasikan sistem extension setelah keputusan desain ditinjau

**Catatan:** belum ada komitmen pada format, bahasa, marketplace, maupun model
eksekusi extension.

## Prinsip penerapan

- Kerjakan satu tahap dalam lingkup kecil dan dapat diuji.
- Utamakan penyimpanan data yang aman, error yang terlihat, dan UI responsif.
- Hindari menambahkan dependensi sebelum kebutuhan serta trade-off-nya jelas.
- Verifikasi perubahan dengan tes crate terkait dan pemeriksaan build aplikasi.
