**Phase 7 — Autonomous Desktop Pet**

Status: roadmap dengan implementasi awal berjalan opsional. Checkbox **Aktifkan
jalan** (default mati, tersimpan) memakai `animasi_basic/Walking.vrma` untuk jalan
singkat di lantai monitor saat ini. Kontrol radius/tingkat aktivitas, rutinitas
lanjutan, dan berjalan di window belum diimplementasikan. Uji visual Windows dan
profiling pemakaian panjang tetap diperlukan; tahap-tahap di bawah adalah target
penyelesaian, bukan klaim semuanya sudah selesai.

Tujuannya: Nera bisa memilih aktivitas ringan, berjalan sebentar, berhenti, dan
beristirahat sendiri, sambil tetap mudah dikendalikan dan tidak mengganggu pekerjaan.
Contoh hasil akhirnya: setelah diam beberapa saat Nera berjalan pelan, berhenti
sebelum tepi layar, melihat sekitar, lalu istirahat. Saat pengguna menyeretnya,
gerakan otomatis langsung berhenti; posisi hasil drag menjadi titik aktivitas baru.

**Fondasi yang sudah tersedia**

Click-through, drag, headpat, reaksi klik, duduk pada window/taskbar, gravitasi,
landing, gaze, alarm, settings, dan pemulihan posisi sudah ada. Empat idle variation
sudah berjalan otomatis ketika Nera idle. Animation Manager sudah memiliki
bookmark, Combine, dan proteksi 17 file aktif di `animasi_basic`, termasuk Walking.

Controller awal sudah menangani jeda, tujuan horizontal, dan pembatalan jalan.
Pemilih aktivitas mandiri yang lebih luas belum ada.
Follow Mouse sekarang mengatur pandangan, bukan membuat Nera berjalan mengikuti
mouse. Physics saat ini terutama menangani gerak vertikal.

**Urutan pengerjaan**

| Tahap | Hasil yang bisa dicoba | Kriteria selesai |
|---|---|---|
| 7.0 — Baseline dan aset | Catatan RAM/CPU release, daftar klip untuk berjalan/berputar/istirahat, serta aturan interupsi | Kandidat diuji pada rig Nera; loop, arah hadap, gerakan root dan kaki diperiksa; masalah lama yang relevan direproduksi ulang sebelum dijadikan blocker |
| 7.1 — Controller perilaku | Nera memilih Idle, Walk, Pause, atau Rest berdasarkan kondisi dan cooldown | Satu keputusan aktif; drag, reaksi pengguna, jatuh, menu, alarm, dan animasi pilihan pengguna bisa menginterupsi; tidak ada intent lama yang tiba-tiba melanjutkan |
| 7.2 — Berjalan di permukaan aman | Jalan kiri/kanan di lantai desktop pada monitor saat ini, dengan radius terbatas | Gerakan HWND cocok dengan animasi; tidak bergeser saat idle, keluar layar, atau bergerak dua kali akibat root motion; berhenti/berputar sebelum batas aman |
| 7.3 — Kontrol pengguna | Stay Here, Allow Roaming, Pause, tingkat aktivitas, dan radius jelajah | Kontrol tersimpan; drag menetapkan titik jelajah baru; membuka menu menghentikan gerak; Hide dan sleep/resume tidak menghasilkan loncatan atau antrean aktivitas |
| 7.4 — Rutinitas tenang | Siklus eksplorasi singkat, melihat sekitar, istirahat, dan variasi ketika duduk | Tidak mengulang aksi yang sama berturut-turut; klip berdiri tidak menarik Nera dari kursi; tidur/bangun hanya ditambahkan setelah klip dan transisinya lolos verifikasi |
| 7.5 — Permukaan window | Berjalan terbatas di tepi window yang sedang menopang Nera | Offset attachment ikut diperbarui; resize, minimize, close, dan pergantian DPI tidak membuat Nera tersentak, tertinggal, atau mempertahankan support yang sudah hilang |
| 7.6 — Uji pemakaian panjang | Versi Phase 7 siap dipakai sehari-hari | Lulus pengujian interupsi, multi-monitor, pemulihan posisi, dan pemakaian 60 menit; tidak ada pertumbuhan memori terus-menerus setelah warm-up |

MVP pertama berhenti pada **7.0–7.3**: berjalan mandiri yang aman dan bisa dimatikan
lebih dahulu. Rutinitas kaya dan jelajah tepi window menyusul setelah MVP stabil.
Perpindahan monitor otomatis, lompatan antarwindow, mood persisten, makanan,
minigame, serta chat/AI berada di backlog lanjutan.

**Aturan perilaku dan desktop**

- Aktivitas otomatis hanya berjalan ketika kondisi placement, physics, dan
  animation mengizinkan. Prioritas existing tetap dipertahankan; otonomi menjadi
  aktivitas berprioritas rendah, bukan controller animasi kedua.
- Jeda keputusan awal 2–5 Hz; interpolasi gerak tetap mengikuti frame. Angka ini
  adalah titik awal tuning, bukan janji performa.
- Sediakan cooldown, durasi maksimum aktivitas, dan penghindaran pengulangan.
  Setelah interupsi, hitung ulang kondisi; jangan melanjutkan tujuan lama secara buta.
- MVP terbatas pada monitor saat ini. Gunakan batas work area dan ukuran avatar,
  bukan angka lebar layar tetap. Berhenti ketika pengguna mulai berinteraksi agar
  Nera mudah diklik atau diseret.
- Berikan opsi jeda ketika aplikasi fullscreen aktif. Jeda roaming tidak boleh
  diam-diam mematikan alarm. Jangan merebut fokus keyboard atau menggerakkan
  window aplikasi milik pengguna.
- Saat tersembunyi atau sistem suspend, hentikan scheduler. Resume melakukan
  validasi monitor/support dan reset cooldown; waktu offline tidak dibayar dengan
  rentetan aktivitas.
- Follow Mouse tetap berarti gaze. Gerakan menuju kursor, jika ditambahkan nanti,
  memakai kontrol terpisah yang jelas.

**Pembagian tanggung jawab implementasi**

Usulan modul `pet_behavior` menyimpan tujuan, cooldown, dan pilihan aktivitas.
Modul itu hanya mengirim intent; ia tidak memanggil Win32 untuk memindahkan window.

Pergerakan harus memiliki satu pemilik per frame: drag, attention, attachment,
physics, atau locomotion. Jalur locomotion diintegrasikan dengan placement/physics
yang ada. Khusus berjalan pada window, offset support harus ikut berubah supaya
mekanisme follow tidak menarik Nera kembali ke posisi lama.

[Animation state](C:/Users/sahid/Documents/ai/src/animation/state.rs) tetap menjadi
otoritas pose. [Desktop physics](C:/Users/sahid/Documents/ai/src/desktop_physics.rs)
dan [sitting](C:/Users/sahid/Documents/ai/src/environment/sitting.rs) tetap memiliki
support/attachment. [Presentation](C:/Users/sahid/Documents/ai/src/presentation.rs)
tetap menguasai posisi saat alarm Attention. Settings menyimpan preferensi dan
posisi secara tertunda, bukan menulis ke disk pada setiap langkah berjalan.

Scheduler idle lama perlu disatukan dengan pemilih aktivitas baru agar dua timer
tidak saling memulai animasi. Ekspresi tetap mengikuti pose yang benar-benar menang.

**Kebutuhan animasi dan proteksi**

- Minimal: walk loop yang cocok dengan rig, arah hadap yang benar, dan transisi
  berhenti yang layak. Kandidat `Walk_Loop`, `Walk_Formal_Loop`, dan klip Walking
  sudah ada dalam koleksi, tetapi nama file belum membuktikan kecocokan.
- Putuskan cara menangani root motion sebelum menentukan kecepatan HWND agar
  karakter tidak meluncur atau bergerak dua kali.
- Klip tidur/bangun berbeda dari fidget `Sleepy`; jangan menganggap fidget itu
  sebagai siklus tidur yang lengkap.
- Hanya klip yang benar-benar dipasang pada perilaku aktif yang masuk
  `animasi_basic` dan katalog protection. `Sitting_Exit` tetap stok selama belum
  ada pemicu aktif yang memakainya. Jangan memasukkan seluruh koleksi sebagai aset dasar.
- Klip gagal dimuat harus menonaktifkan aktivitas terkait dan kembali ke idle;
  scheduler tidak boleh macet menunggu satu aset.

**Performa dan verifikasi**

Gunakan kumpulan klip perilaku yang kecil dan terukur. Cache playback desktop saat
ini menyimpan klip yang pernah dimainkan; ukur dan tentukan kebijakan batasnya
sebelum menghubungkan scheduler dengan banyak animasi koleksi. Pelepasan cache
harus menjaga konsistensi graph VRM, bukan sekadar menghapus entity secara acak.

Catat private bytes/RAM, CPU, dan frame time pada build release untuk proses pet,
proses Animation Manager, serta total keduanya. Target idle **100–300 MB** tetap
merupakan sasaran optimasi yang perlu dibuktikan lewat profiling, bukan jaminan
hasil Phase 7. Kriteria minimum adalah tidak ada pertumbuhan terus-menerus pada
beban yang sudah stabil dan tidak ada kebocoran siklus buka/tutup manager.

Pengujian wajib mencakup:

1. Idle → walk → pause → idle, dengan seed acak tetap untuk tes keputusan.
2. Drag, headpat, menu, pilihan animasi, dan alarm ketika Nera sedang berjalan.
3. Batas layar, DPI 100/125/150%, monitor dengan koordinat negatif, dan perubahan ukuran avatar.
4. Support bergerak/hilang, Hide/Show, suspend/resume, serta Reset Position.
5. Klip hilang/rusak dan animasi yang belum siap: fallback tetap responsif.
6. Pemakaian 60 menit dan 20 siklus buka/tutup manager, termasuk memilih beberapa
   animasi desktop; bandingkan dengan baseline pada kondisi render yang sama.

Tes otomatis memeriksa keputusan, interupsi, dan geometri. Perilaku HWND,
click-through, DPI, arah hadap, dan kenyamanan gerakan tetap memerlukan uji visual
Windows. Pengujian visual dijalankan pengguna; perubahan awal tidak menjalankan Nera.
