# Phase 7 — Autonomous Desktop Pet

Status: fondasi behavior, movement, dan kontrol pengguna sudah diimplementasikan
memakai aset yang tersedia. `cargo check` berhasil;
`cargo test`: **229 passed, 0 failed, 5 ignored** (tes native/aset lokal opt-in existing).
Binary debug terbaru selesai dikompilasi dan di-link. `cargo build` gagal menimpa
`nera.exe` yang masih berjalan; hasil compiler disalin dan diverifikasi sebagai
`target/debug/nera-updated.exe`. Tutup Nera lama sebelum membuka build tersebut.
Uji visual Windows dan profiling pemakaian panjang belum
dijalankan; implementasi kode bukan klaim seluruh kriteria desktop sudah terbukti.

Tujuannya: Nera memilih aktivitas ringan, berjalan sebentar, berhenti, dan
beristirahat sendiri, sambil tetap mudah dikendalikan. **Allow Roaming default
mati**, termasuk profil lama. Mengaktifkannya mengizinkan jalan otomatis saat
berdiri; posisi duduk/berdiri mengikuti penempatan pengguna dan support fisik.
Interaksi dan variasi idle tetap tersedia ketika roaming
dimatikan.

## Implementasi saat ini

| Tahap | Implementasi | Batas dan verifikasi berikutnya |
|---|---|---|
| 7.0 — Behavior Controller | PetBehavior memilih StayIdle, variasi idle sesuai pose, atau Walk; satu pemilik keputusan menggantikan scheduler idle/jalan terpisah | Controller hanya membaca placement; tidak mendudukkan Nera atau melepas attachment duduk |
| 7.1 — Home Anchor + Radius | Drag menetapkan home baru; radius 300–600 logical px, default 400, dikonversi menurut DPI monitor | Home/support/geometry divalidasi kembali; uji drag dan mixed-DPI native masih diperlukan |
| 7.2 — Activity + Anti-Repetition | Calm/Normal/Active menentukan cooldown jalan; tiga pilihan terakhir memengaruhi pemilihan berbobot | Cooldown menghitung waktu eligible, bukan janji jalan pada detik tertentu |
| 7.3 — Smooth Walk / Stop / Turn | Tujuan target_x, LookAround sebelum setiap perjalanan, variasi jarak/kecepatan, jeda hadap 200–400 ms, percepatan dan pengereman | Kualitas gait, arah hadap, dan kemungkinan foot sliding perlu dilihat pada rig Nera |
| 7.4 — Edge Awareness | Batas support/monitor/home dipotong menjadi interval aman; setelah mencapai batas, jeda lalu tujuan kembali ke dalam | Perubahan support atau gangguan membatalkan tujuan lama; tidak memaksa perjalanan lanjut |
| 7.5 — Stay Here + Fullscreen | Submenu Roaming menyediakan Allow Roaming/Stay Here; Pause in Fullscreen default aktif | DND menjeda otonomi; tidak menyembunyikan Nera atau mematikan alarm |
| 7.6 — Window-top Roaming | Berjalan di atas window yang sudah menopang Nera dalam keadaan berdiri; target dan offset support mengikuti window | Bukan berjalan sambil duduk, naik ke window secara otomatis, atau melompat antarwindow |
| 7.7 — Multi-monitor Robustness | Tetap pada monitor saat ini; perpindahan manual, perubahan DPI/ukuran/support membatalkan atau memperbarui home dan intent | Verifikasi visual monitor nyata, koordinat negatif, resize/minimize/close, dan mixed-DPI belum dijalankan |
| 7.8 — Optional Personality | Klip lokal tambahan sudah terintegrasi untuk idle berdiri, idle duduk, dan dodge duduk | Intent opsional lain tetap fallback ke pose dasar |

Walking memakai assets/animations/animasi_basic/Walking.vrma. Setiap perjalanan
menunggu LookAround selesai sebelum jeda hadap dan gerak. Idle berdiri memakai
LookAround, Relax, Sleepy, Blush, Catwalk_Idle_Twist_R, Bored, dan Angry_Gesture.
Setiap mulai duduk, Nera memilih satu posisi dari Sitting_Idle_Loop, Sitting_2,
atau Sitting_4 yang sudah siap, menghindari posisi sebelumnya bila ada alternatif.
Posisi ini di-loop sepanjang sesi duduk dan tidak diganti timer idle. Duduk ulang
memilih lagi; menu, dodge, dan kembali dari alarm mempertahankan pilihan tadi.
Sitting_Rubbing_Arm tetap sebagai gerakan kecil saat duduk. Hit kepala/dada
saat duduk memicu Dodge_While_Sitting tanpa melepas attachment atau menghitung
combo death berdiri. Tujuh klip tambahan dipindahkan dari Bookmarks ke
animasi_basic; menu titik tiga klip dasar disembunyikan, preview tetap tersedia.
Physics, interaksi, alarm, dan prioritas animasi tetap berada pada sistem existing.
Tidak diperlukan hunting VRMA baru untuk fondasi ini.

Perbaikan reliability: semua alarm pada menit yang sama masuk antrean meski quick
alarm/snooze aktif; deadline dan label timer tetap tersimpan. Cleanup alarm lama
selesai sebelum alarm berikutnya dimulai. Alt+F4/native close pada panel alarm
menutup Attention melalui lifecycle biasa. Pemutaran galeri memvalidasi VRMA
sebelum memasangnya ke rig, memakai timeout 20 detik per tahap, dan menampilkan
error di manager tanpa mengunci pilihan berikutnya. Cache desktop dibatasi 32
clip/64 MiB total ukuran file sumber, maksimum 16 MiB per file. Saat penuh, clip
cached tetap bisa dimainkan; clip baru memerlukan restart Nera. Batas ini bukan
hasil pengukuran RAM. Semua HWND proses Nera dan worker manager dikecualikan dari
support/fullscreen; settings lampu hanya mengubah NeraMainLight.

## Kontrol pengguna dan cadence

- **Allow Roaming:** sakelar tersimpan, default mati; tersedia pada Settings dan
  submenu Roaming di radial menu.
- **Stay Here:** mematikan Allow Roaming dan menyimpan pilihan. Nera tetap aktif
  untuk interaksi, gaze, alarm, dan variasi idle. Pergantian preferensi roaming
  tidak mengubah posisi duduk/berdiri.
- **Activity:** Calm memakai cooldown jalan 60–180 detik, Normal 30–90 detik,
  Active 15–45 detik. Normal adalah default.
- **Roaming Distance:** slider 300–600 logical px dari home, default 400.
  Koordinat HWND tetap physical; radius dikalikan scale factor monitor.
- **Pause in Fullscreen:** default aktif; deteksi foreground fullscreen pada
  monitor tempat Nera berada disampling berkala, tanpa merebut fokus aplikasi.

Rentang Activity adalah cooldown selama kondisi eligible, bukan interval
wall-clock yang wajib dipenuhi. Sesudah cooldown habis, controller masih dapat
memilih idle atau variasi. Interaksi dan pause memulai ulang jeda yang aman.
Jarak perjalanan diacak dari kisaran dasar 120–500 px dan disesuaikan DPI/ukuran
karakter, kemudian dibatasi home dan permukaan. Jeda keputusan normal sesudah
perjalanan berada pada rentang 8–30 detik. Tepi memiliki jeda singkat sebelum
perjalanan balik ke dalam.
Interupsi juga membatalkan hasil perjalanan yang belum diproses, sehingga hasil
Edge lama tidak memicu perjalanan balik sesudah pause atau drag.

## Aturan behavior dan desktop

- Satu controller memilih aktivitas otonom berprioritas rendah. Animation state
  tetap menentukan pose yang benar-benar menang; ekspresi mengikuti pose itu.
- Autonomi berhenti saat drag, hover/press, reaksi pengguna, jatuh, alarm Attention,
  menu, Settings/Alarm Settings, Animation Manager terbuka, pet tersembunyi, atau
  fullscreen pause aktif. Intent lama dibatalkan; tidak ada perjalanan tertunda
  yang langsung melanjutkan setelah interupsi.
- Fullscreen/DND menjeda otonomi dan menghapus fidget otonom yang sedang berjalan.
  Alarm tetap dijadwalkan dan boleh masuk Attention sesuai perilaku existing.
- Selama Animation Manager terbuka, pet tidak roaming; pause ini tidak bergantung
  pada lokasi pointer di dalam manager.
- Gerak otomatis tetap pada monitor saat ini. Drag ke monitor lain menetapkan
  home di monitor baru setelah support valid. Perpindahan monitor otomatis tidak
  diimplementasikan.
- Jalan memerlukan standing support dan PlacementState::Free. Duduk memakai
  attachment Seat existing dari penempatan pengguna. Pemilihan Sit otomatis dan
  timer berdiri 12–30 detik dihapus. Nera tetap duduk sampai diseret atau support
  tidak lagi valid. Menu klik kanan tidak memainkan Praying ketika sedang duduk.
- Window-top walking memeriksa support dan calon posisi kaki, termasuk visibility
  dan occlusion. Gerak horizontal memperbarui offset support agar follower physics
  tidak menarik Nera kembali. Support hilang atau berubah ditangani oleh validasi
  physics; aplikasi milik pengguna tidak dipindahkan.
- Jeda frame besar/suspend membatalkan motion. Resume menghitung ulang kondisi dan
  cooldown; waktu offline tidak dibayar dengan rentetan aktivitas atau loncatan.
- Follow Mouse tetap berarti gaze, bukan gerakan menuju kursor.

## Kepemilikan implementasi

[pet_behavior.rs](src/pet_behavior.rs) memiliki home, riwayat, pemilihan berbobot,
cooldown, dan gate interupsi/fullscreen. Ia meminta animasi atau perjalanan melalui
sistem existing. DesktopEnvironment diakses read-only; behavior tidak mengubah
placement atau memindahkan HWND langsung.

[walking.rs](src/walking.rs) memiliki WalkIntent, tujuan X relatif terhadap support,
turn delay, kecepatan/subpixel X, percepatan/pengereman, dan hasil
Arrived/Edge/Interrupted. Modul ini mengeksekusi intent; tidak memiliki timer untuk
memilih perjalanan sendiri.

[desktop_physics.rs](src/desktop_physics.rs) menyediakan interval standing support
dan menerapkan langkah native horizontal melalui walk_horizontal, termasuk
pembaruan offset window. Gravity dan support tetap di physics; sitting attachment
tetap dimiliki [sitting.rs](src/environment/sitting.rs).
[Animation state](src/animation/state.rs) tetap otoritas prioritas pose.
[Presentation](src/presentation.rs) menguasai HWND/camera selama alarm Attention.

main.rs mengurutkan physics dan sinkronisasi interaksi sebelum behavior, lalu
sinkronisasi pose environment, walking, timer animasi, dan playback. Pose mengikuti
attachment environment, bukan keputusan duduk/berdiri dari behavior.
Behavior tetap dipanggil ketika konteks
terblokir agar pembatalan berjalan. Scheduler idle independen lama sudah dihapus.
Settings menyimpan preferensi melalui model yang kompatibel dan save tertunda;
gerak per-frame tidak menulis preferensi roaming ke disk.

## Aset dan perluasan personality

Kosakata optional mencakup Stretch, Yawn, LookAround, SitFidget, Turn, StandUp,
dan SitDown. Saat ini hanya **LookAround** memiliki pemetaan capability ke idle
clip existing. Intent lain belum menjadi rutinitas aktif dan tidak mencari atau
mengimpor file baru secara otomatis. Permintaan tanpa capability kembali aman
tanpa mengambil alih pose atau menunggu aset selamanya.

Klip walking/variasi idle harus siap dimuat sebelum behavior terkait menjadi
pilihan. Kandidat yang tidak siap dilewati dan StayIdle tetap menjadi fallback.
Sleepy adalah fidget, bukan siklus tidur/bangun lengkap.

Koleksi aktif di animasi_basic tetap diproteksi. Hanya klip yang benar-benar
diintegrasikan boleh ditambahkan sebagai aset dasar; Sitting_Exit dan kandidat
lain tidak dianggap aktif hanya karena file tersedia. Sebelum menambah klip,
periksa kompatibilitas rig, loop, root motion, arah hadap, dan transisi pada Nera.

## Verifikasi yang tersisa

Tes otomatis memeriksa keputusan berbasis seed, anti-repetition, cooldown,
settings lama/batas nilai, interupsi, geometri tujuan/support, dan motion tanpa
overshoot. Hasil terintegrasi: `cargo check` berhasil, binary debug tersedia seperti
catatan di atas, dan
`cargo test` **229 passed, 0 failed, 5 ignored**.
Tes tersebut tidak membuktikan kualitas gerak dan perilaku HWND nyata.

Pengujian visual dijalankan pengguna; pekerjaan implementasi ini **tidak
menjalankan Nera**. Checklist native berikut masih perlu dilakukan:

1. Allow Roaming on/off, Stay Here, Activity, radius, reset default, dan persistensi
   sesudah restart; memastikan roaming tetap mati pada profil lama.
2. Idle → LookAround selesai → hadap → jalan → berhenti; batas home/layar, jeda lalu balik ke dalam,
   variasi jarak, dan kenyamanan gait tanpa foot sliding atau root-motion ganda.
3. Drag, headpat, menu, animasi pilihan, alarm, fullscreen, Settings, dan manager
   ketika berjalan; pastikan alarm tetap aktif selama DND.
4. Window support bergerak/resize/minimize/close/cloak/occlusion, taskbar, perubahan
   ukuran avatar, serta duduk/berdiri tanpa konflik penempatan.
5. DPI 100/125/150%, monitor berkoordinat negatif, drag antar-monitor, Hide/Show,
   suspend/resume, dan Reset Position. Belum ada klaim lulus multi-monitor nyata.
6. Aset hilang/rusak/tidak siap: fallback responsif dan pose lain tidak tertahan.
   Periksa variasi duduk, dodge saat kepala/dada diklik, dan kontak duduk tetap
   stabil; menu titik tiga hanya muncul pada animasi non-dasar.
7. Pemakaian 60 menit dan 20 siklus buka/tutup manager, termasuk memilih beberapa
   animasi desktop; bandingkan RAM, CPU, dan frame time dengan baseline release.
8. Dua alarm jatuh tempo pada menit sama saat quick alarm/snooze aktif; tutup panel
   pertama lewat Alt+F4 dan pastikan panel berikutnya tampil serta timer lama tetap
   tersedia. Pilih VRMA rusak, lalu file valid; ulangi sampai batas cache dan
   pastikan error terlihat serta clip cached tetap bisa dimainkan.

Cache playback menyimpan klip yang pernah dimainkan sampai batas admission di atas.
Tes 400 permintaan pada cache penuh memeriksa jumlah entity dan replay, bukan
profiling RAM aplikasi native selama 30–60 menit. Sebelum memperluas koleksi
otomatis, ukur private bytes/RAM pet, manager, serta total keduanya, dan periksa
stabilitas setelah warm-up. Target idle **100–300 MB** adalah sasaran profiling,
bukan jaminan hasil Phase 7. Pelepasan cache perlu menjaga konsistensi graph VRM.

Rutinitas tidur/bangun lengkap, perpindahan monitor otomatis, lompatan antarwindow,
mood persisten, makanan, minigame, dan chat/AI tetap backlog.
