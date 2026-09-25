# Audit bug Project Nera

## Ringkasan

Audit mencakup alur animasi/preview, filesystem galeri, input, physics dan sitting,
alarm, presentation, settings, tray, serta integrasi Win32. Inventaris repo:
37 berkas Rust, satu shader lokal, dan konfigurasi build.

**9 temuan berhasil direproduksi melalui 7 skenario audit tambahan.** Suite
bawaan berisi 122 tes tetap lulus, termasuk pengujian model dan VRMA asli.
Gabungan pengujian: **129 lulus, 0 gagal**. Tes audit sengaja memeriksa bahwa
kondisi bug terjadi; hasil lulus tersebut bukan tanda bug sudah diperbaiki.

Ada juga **5 risiko dari penelusuran kode** yang memerlukan pengujian GUI atau
profiling untuk mengukur dampaknya. Audit ini tidak menjamin seluruh bug sudah
ditemukan, terutama untuk kombinasi monitor/DPI, sleep/resume, dan native window.

Tidak ada perbaikan perilaku runtime dalam audit ini. Snippet tes sementara
dikembalikan ke isi sumber semula setelah pengujian; fixture aset juga dibersihkan.
`cargo check` tidak dijalankan.

- [Log pengujian](C:/Users/sahid/Documents/ai/target/bug-audit/results.log)
- [Harness reproduksi dan pemulihan sumber](C:/Users/sahid/Documents/ai/target/bug-audit/run-audit.ps1)

P1 = perlu diprioritaskan karena fungsi utama rusak atau peristiwa hilang.
P2 = perlu diperbaiki karena menghasilkan perilaku salah dalam kondisi tertentu.

## Temuan yang direproduksi

### 1. [P1] Galeri dapat menghapus atau mengganti nama aset wajib aplikasi

**Lokasi:** [registry.rs:164](C:/Users/sahid/Documents/ai/src/animation_manager/registry.rs:164),
[rename:114](C:/Users/sahid/Documents/ai/src/animation_manager/registry.rs:114),
[jalur produksi](C:/Users/sahid/Documents/ai/src/animation/playback.rs:146).

Galeri memasukkan `animasi_basic` ke katalog dan menyediakan Rename/Delete untuk
semua entri. Controller masih menggunakan jalur tetap untuk idle, duduk, reaksi,
dan alarm. Tidak ada pemeriksaan apakah sebuah file sedang menjadi dependensi
perilaku bawaan.

**Pemicu:** hapus atau rename `animasi_basic/idle.vrma` melalui galeri, kemudian
restart aplikasi. Clip yang sudah dimuat dapat tetap hidup sampai restart,
sehingga kerusakan awalnya tersembunyi. Setelah restart, permintaan jalur lama
gagal. File bawaan yang gagal dimuat juga dapat menahan loading galeri desktop.

**Bukti:** fixture katalog dengan jalur `animasi_basic/idle.vrma` berhasil dihapus
oleh API registry. Aset asli pengguna tidak dihapus saat audit.

**Perbaikan:** tandai dependensi produksi sebagai aset terlindungi, atau sediakan
mekanisme penggantian referensi dan fallback sebelum mutasi fisik dilakukan.

### 2. [P1] Quick Alarm atau snooze memblokir alarm terjadwal

**Lokasi:** [alarm.rs:493](C:/Users/sahid/Documents/ai/src/alarm.rs:493).

Scheduler alarm tersimpan langsung kembali jika `AlarmState` bukan `Idle`.
Quick Alarm selama dua jam membuat jadwal lain tidak diperiksa selama dua jam.
Setelah itu, pencocokan hanya menggunakan menit saat ini; jadwal yang sudah
terlewat tidak masuk antrean. Alarm yang sedang berbunyi dan snooze juga
menimbulkan masalah yang sama jika melewati menit jatuh tempo alarm berikutnya.

**Bukti:** jadwal yang jatuh tempo sekarang tidak pernah diperiksa ketika state
diisi timer aktif; `last_checked` tetap kosong dan tidak ada alarm ditandai due.

**Perbaikan:** pisahkan deteksi jadwal jatuh tempo dari satu presentasi alarm
aktif. Simpan due event dalam antrean dengan aturan penggabungan/prioritas yang
jelas, termasuk kebijakan jadwal terlewat setelah aplikasi tersendat atau resume.

### 3. [P1] Dua alarm mingguan pada menit yang sama kehilangan alarm kedua

**Lokasi:** [alarm.rs:512](C:/Users/sahid/Documents/ai/src/alarm.rs:512),
[pemilihan satu alarm:528](C:/Users/sahid/Documents/ai/src/alarm.rs:528).

Scheduler menyimpan `last_checked` untuk menit tersebut, lalu memakai `.position()`
untuk mengambil hanya satu alarm. Alarm mingguan pertama tidak mengubah alarm
book. Sesudah alarm pertama dismissed pada menit yang sama, guard `last_checked`
melewati seluruh pemeriksaan; alarm kedua tidak dijadwalkan.

**Bukti:** dua alarm mingguan identik waktunya; setelah trigger pertama dan
simulasi dismiss, jumlah `last_fired` tetap satu dan state tetap `Idle`.

**Perbaikan:** kumpulkan seluruh due alarm sebelum menandai satu menit selesai
diperiksa. Deduplicasi berdasarkan ID alarm dan waktu kejadian.

### 4. [P1] Menutup panel alarm melalui OS dapat meninggalkan alarm tanpa tombol dismiss

**Lokasi:** [alarm_dismiss.rs:808](C:/Users/sahid/Documents/ai/src/alarm_dismiss.rs:808),
[cleanup:851](C:/Users/sahid/Documents/ai/src/alarm_dismiss.rs:851).

Penutupan native, misalnya Alt+F4 ketika panel memiliki fokus, menghapus entity
window melalui WindowPlugin. Runtime masih menyimpan `window: Some(...)` dan
`revealed: true`. Failsafe tidak bekerja setelah `revealed`, sedangkan cleanup
hanya menangani presentation yang sudah keluar. Tidak ada pengamat hilangnya
panel yang meminta `ExitAttention`.

**Bukti:** simulasi penghapusan entity panel setelah revealed meninggalkan
presentation dalam Attention, referensi window mati, dan tidak mengirim perintah
keluar. Ini reproduksi lifecycle ECS; tombol Alt+F4 native belum diuji langsung.

**Perbaikan:** tangani `WindowCloseRequested` atau hilangnya `AlarmDismissWindow`
sebagai jalur dismiss yang lengkap, termasuk penghentian audio dan animasi.

### 5. [P2] File yang berhasil dibaca tetapi gagal inisialisasi VRMA memacetkan antrean

**Lokasi:** [library.rs:57](C:/Users/sahid/Documents/ai/src/animation/library.rs:57),
[guard loading:100](C:/Users/sahid/Documents/ai/src/animation/library.rs:100),
[import:175](C:/Users/sahid/Documents/ai/src/animation_manager/registry.rs:175).

Import hanya memeriksa ekstensi file. File glTF yang valid tetapi tidak memiliki
extension animasi VRM bisa berstatus asset `Loaded`. Inisialisasi dependency
kemudian berhenti tanpa mengirim `LoadedVrma`. Runtime hanya menyelesaikan
loading ketika asset `Failed` atau ketika `LibraryClipReady` hadir, sehingga
`loading` tidak pernah kosong.

**Pemicu:** import file tersebut, klik untuk desktop, lalu pilih animasi valid
yang belum dicache. Permintaan valid berikutnya tertahan. Clip yang sudah
dicache dan perilaku bawaan masih bisa bekerja.

**Bukti:** fixture glTF valid tanpa extension VRMA mencapai `Loaded`, gagal
inisialisasi, dan tetap menahan permintaan `Relax.vrma` setelah 30 update.

**Perbaikan:** validasi isi file saat import dan tangani kegagalan inisialisasi
secara eksplisit. Tambahkan batas waktu loading serta error yang terlihat di UI.

### 6. [P2] Mengganti file pada path yang sama tetap memainkan clip lama

**Lokasi:** [library.rs:77](C:/Users/sahid/Documents/ai/src/animation/library.rs:77).

Cache hanya memakai path. Rename/delete/import di registry tidak menginvalidasi
cache desktop maupun memaksa reload asset. Setelah file pernah dimainkan,
menghapusnya lalu mengimpor file berbeda dengan nama yang sama dapat membuat
galeri menunjuk file baru sementara desktop memakai entity dan clip lama.

**Bukti:** fixture Angry dimainkan, file diganti dengan Waving, lalu path yang
sama diminta lagi. Entity dan durasi clip tetap milik pemuatan pertama.

**Perbaikan:** kirim event mutasi aset dari registry; gunakan generasi/fingerprint
file dan reload eksplisit. Pertahankan konsistensi node graph saat mengganti clip.

### 7. [P2] Rename/delete bisa berhasil secara fisik tetapi dilaporkan gagal dan katalog tertinggal

**Lokasi:** [registry.rs:150](C:/Users/sahid/Documents/ai/src/animation_manager/registry.rs:150),
[penyimpanan bookmark:158](C:/Users/sahid/Documents/ai/src/animation_manager/registry.rs:158).

Operasi filesystem dilakukan lebih dahulu. Jika penyimpanan bookmark gagal,
operator `?` keluar sebelum `refresh()`. UI mendapatkan error, tetapi file sudah
dipindahkan atau dihapus. Katalog masih menunjuk nama/path lama.

**Bukti:** penulisan bookmark sengaja dibuat gagal memakai fixture direktori
yang tidak valid. Rename mengembalikan error, file `New.vrma` sudah ada, file lama
hilang, dan registry masih menganggap `Old.vrma` sebagai entri yang valid.

**Perbaikan:** pastikan katalog disinkronkan setelah mutasi fisik, dan pisahkan
hasil operasi file dari kegagalan metadata. Jika transaksi harus atomik, siapkan
rollback yang juga menangani kegagalan rollback.

### 8. [P2] Gerakan sementara saat alarm menimpa posisi desktop yang disimpan

**Lokasi:** [runtime.rs:165](C:/Users/sahid/Documents/ai/src/settings/runtime.rs:165),
[penulisan koordinat:190](C:/Users/sahid/Documents/ai/src/settings/runtime.rs:190).

Semua `WindowMoved` primary window ditulis ke settings, termasuk animasi masuk
dan posisi portrait Attention. Ketika alarm ditutup normal, posisi biasanya
dikembalikan. Namun Exit aplikasi saat alarm masih aktif dapat menyimpan posisi
sementara itu melalui penyimpanan sinkron.

**Bukti:** event gerak primary window saat PresentationState berada dalam
Attention mengganti koordinat tersimpan dan mengaktifkan flag penyimpanan.

**Perbaikan:** simpan koordinat hanya ketika DesktopPet memiliki penempatan,
atau gunakan snapshot posisi desktop selama Attention.

### 9. [P2] Pengaturan shadow desktop ikut menyalakan shadow untuk seluruh preview

**Lokasi:** [shadow.rs:385](C:/Users/sahid/Documents/ai/src/settings/shadow.rs:385),
[query semua lampu:390](C:/Users/sahid/Documents/ai/src/settings/shadow.rs:390).

Preview membuat lampu dengan shadow nonaktif. `apply_light_settings` kemudian
memodifikasi semua `DirectionalLight` setiap update, tanpa penanda lampu desktop.
Ketika Character Shadow aktif dan desktop pet grounded, shadow preview ikut
menyala; render layer tidak membatasi query ECS ini.

**Bukti:** lampu pada render layer preview, awalnya tanpa shadow, berubah menjadi
`shadow_maps_enabled = true` setelah satu update sistem settings.

**Perbaikan:** beri marker lampu desktop dan filter query. Ukur kembali beban
GPU/VRAM untuk galeri 15 preview; besarnya penurunan performa belum diukur.

## Risiko tambahan dari pembacaan kode

Temuan berikut belum direproduksi melalui native GUI atau profiling; jangan
anggap dampak runtime-nya sudah terukur.

1. **Cache desktop tidak dibatasi.** [library.rs:112](C:/Users/sahid/Documents/ai/src/animation/library.rs:112)
   menyimpan setiap clip yang pernah dipilih sampai aplikasi berakhir. Menutup
   galeri hanya membersihkan pool preview. Repo memiliki 227 file VRMA; impor
   tambahan memperbesar jumlah potensial. Perlu ukur pertumbuhan entity/RAM dan
   merancang pelepasan clip yang menjaga node graph tetap konsisten.
2. **Always On Top OFF berpotensi diabaikan saat duduk di taskbar.**
   [sitting.rs:1043](C:/Users/sahid/Documents/ai/src/environment/sitting.rs:1043)
   memanggil `HWND_TOPMOST` tanpa memeriksa pengaturan pengguna. Jalur duduk pada
   window biasa sudah membaca window level. Uji toggle OFF, duduk di taskbar,
   kemudian pindah ke aplikasi lain.
3. **Standing support dapat lepas ketika window dipindah jauh secara horizontal.**
   [geometry.rs:140](C:/Users/sahid/Documents/ai/src/desktop_physics/geometry.rs:140)
   memeriksa koordinat kaki lama terhadap rectangle window baru sebelum memakai
   offset follow. Uji window yang sedang menopang kaki dipindahkan melewati
   koordinat kaki lama dalam satu update. Berlaku untuk standing support;
   sitting menggunakan jalur berbeda.
4. **Jendela utility milik Nera bisa dianggap sebagai window eksternal.**
   [windows.rs:388](C:/Users/sahid/Documents/ai/src/platform/windows.rs:388)
   hanya mengecualikan HWND utama; belum ada pengecualian seluruh window milik
   proses Nera. Utility tanpa `WS_EX_TOOLWINDOW` berpotensi menjadi support atau
   occluder bagi pet sendiri. Perlu uji membuka Settings/Animations di atas
   surface tempat Nera berdiri atau duduk.
5. **Dialog Import memblokir update aplikasi.**
   [animation_manager.rs:339](C:/Users/sahid/Documents/ai/src/animation_manager.rs:339)
   memanggil `GetOpenFileNameW` secara sinkron dari sistem Update. Selama dialog
   native terbuka, alur ECS yang sedang memanggilnya belum bisa menyelesaikan
   frame. Uji animasi dan alarm yang jatuh tempo ketika dialog dibiarkan terbuka;
   pertimbangkan pemilihan file async dengan hasil dikirim kembali ke ECS.

## Urutan perbaikan

1. Lindungi aset produksi dan perbaiki lifecycle penutupan panel alarm.
2. Pisahkan antrean due alarm dari presentation aktif; tangani seluruh due event.
3. Pulihkan loading VRMA yang gagal dan invalidasi cache setelah mutasi file.
4. Rapikan konsistensi registry/bookmark, posisi persisten, dan query lampu.
5. Uji native multi-window, taskbar, perpindahan monitor/DPI, serta profil memori.

Audit tidak melakukan mutasi registry Windows, penghapusan aset asli, penulisan
pengaturan pengguna, atau pengiriman pesan kepada pihak lain.
