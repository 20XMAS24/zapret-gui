---
name: install-local
description: Replace the user's own D:\zapret_GUI\ZapretGUI.exe with a freshly built one (no installer, no release). Use when the user says "замени exe", "поставь мне новую сборку", "я закрыл, заменяй", or wants to try a build before releasing it.
---

# Замена локального ZapretGUI.exe

1. Собрать: `cargo build --release` в `D:\zapret_GUI\ZapretGUI-rs` с `ZAPRET_GUI_REPO=20XMAS24/zapret-gui`
   (PATH — см. `CLAUDE.md`). Без этой переменной exe всё равно смотрит в 20XMAS24/zapret-gui, но лучше явно.
2. Проверить, что программа закрыта:
   `Get-Process ZapretGUI -ErrorAction SilentlyContinue`.
   Если запущена — **попросить пользователя закрыть её самому** (выход из трея) и ждать ответа.
   Не убивать процесс: при работающем прокси он держит правила kill switch (WFP) и TUN-адаптер,
   и никогда не фильтровать taskkill по заголовку окна.
3. Бэкап: скопировать текущий `D:\zapret_GUI\ZapretGUI.exe` в
   `D:\zapret_GUI\_backup\ZapretGUI-rust-pre<новая версия>.exe` (не перезаписывать существующие бэкапы).
4. Скопировать `target\release\ZapretGUI.exe` → `D:\zapret_GUI\ZapretGUI.exe`, сверить размер.
5. Ничего другого в `D:\zapret_GUI` не трогать: `ZapretGUI.ini`, `ZapretGUI.proxy.json`, `lists\`, `bin\` — данные пользователя.
6. Сказать пользователю, что можно запускать, где бэкап, и что изменилось. Окно «Что нового» появится,
   только если версия в exe новее `LastVersion` в `ZapretGUI.ini` и для неё есть текст релиза.
