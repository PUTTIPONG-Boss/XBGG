# XBGG

โหมดคอนโซลแบบเต็มจอสำหรับ Windows เขียนด้วย Rust + [Slint](https://slint.dev)
เป้าหมายคือคืนทรัพยากรเครื่อง (RAM / VRAM / CPU) ให้เกมให้มากที่สุด

## สถานะ: เฟส 1 — Game Library

- สแกนเกมที่ติดตั้งจาก **Steam** (ทุก library folder, พร้อมภาพปก/ภาพพื้นหลังจาก cache ของ Steam), **Epic Games**, **GOG**
- เพิ่มเกม/โปรแกรมเองได้ (`.exe`, `.lnk`, `.url`, `.bat`, `.cmd`)
- ซ่อนเกมที่สแกนเจอ / ลบรายการที่เพิ่มเอง (ไม่ลบไฟล์จริง)
- เรียงตามเล่นล่าสุด
- ใช้ได้ทั้งจอย, คีย์บอร์ด และเมาส์
- ปุ่มจอยจะไม่ทำงานตอนหน้าต่างอื่น (เช่นเกม) อยู่ข้างหน้า

## การควบคุม

| จอย | คีย์บอร์ด | ทำอะไร |
|---|---|---|
| D-pad / สติ๊กซ้าย | ← → | เลื่อน |
| LB / RB | PgUp / PgDn | ข้ามทีละ 5 |
| A | Enter / Space | เล่น / เพิ่ม |
| X | Insert | เพิ่มเกมหรือโปรแกรม |
| Y | Delete | ลบ / ซ่อน |
| View | F5 | สแกนใหม่ |
| B | Esc | ออก (มีถามยืนยัน) |

## Build & Run

```powershell
cargo run --release              # เต็มจอ
cargo run --release -- --windowed
cargo run --release -- --list    # พิมพ์รายการเกมที่สแกนเจอ
cargo test
```

ไฟล์ที่ได้: `target\release\xbgg.exe`

## ข้อมูลผู้ใช้

`%APPDATA%\XBGG\library.json` — รายการที่เพิ่มเอง, รายการที่ซ่อน, เวลาเล่นล่าสุด
(ถ้าอยากเอาเกมที่ซ่อนกลับมา ลบ id ออกจาก `hidden` แล้วกด F5)

## โครงสร้าง

```
crates/
  xbgg-core/   สแกนเกม (steam/epic/gog), เก็บ library, launch
  xbgg/        UI (Slint), จอย (gilrs), โหลดภาพพื้นหลังแบบ lazy
```

## Roadmap

1. ✅ Game library + เพิ่มโปรแกรมเอง + กดเล่น
2. Boost engine: ปิด/suspend แอปพื้นหลัง, หยุด service ชั่วคราว, power plan,
   priority ของเกม, restore journal (คืนค่าได้แม้เครื่องดับ), service ที่มีสิทธิ์ admin
3. Shell mode: บูตเข้า XBGG แทน desktop, UI ปลดตัวเองออกจาก memory ตอนเล่นเกม
4. หน้าสรุปทรัพยากรที่คืนได้ + benchmark (PresentMon)
