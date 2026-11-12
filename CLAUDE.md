# ZapretGUI (Rust) — заметки для Claude

GUI для zapret (winws.exe) + TgWsProxy + системный прокси-туннель (sing-box TUN) с kill switch на WFP.
Пользовательское описание и таблица модулей — в `README.md`. Здесь только то, что не видно из кода.

## Где что лежит
- Репозиторий: `D:\zapret_GUI\ZapretGUI-rs` → https://github.com/20XMAS24/zapret-gui (public, ветка main).
- Рабочая копия пользователя: `D:\zapret_GUI\ZapretGUI.exe` (запускается из папки zapret, рядом `ZapretGUI.ini`,
  `ZapretGUI.proxy.json`, `bin\sing-box.exe`, `lists\`). Бэкапы старых exe — `D:\zapret_GUI\_backup\`.
- Старый C#-вариант — `D:\zapret_GUI\ZapretGUI\` (не трогать, только справочно).
- `dist\` (в .gitignore): установщики, `.sha256`, `notes-<ver>.md` с текстами релизов, кэш загрузок.
- Конфиг TgWsProxy: `%APPDATA%\TgWsProxy\config.json`, лог там же `proxy.log`.

## Сборка (Windows, GNU toolchain, без Visual Studio)
- PATH: `%USERPROFILE%\.cargo\bin` и `...\stable-x86_64-pc-windows-gnu\lib\rustlib\x86_64-pc-windows-gnu\bin\self-contained`
  (см. `build.bat`). Для выпуска на PATH ещё `C:\Program Files\GitHub CLI`; Inno Setup 6 — в `%LOCALAPPDATA%\Programs\Inno Setup 6`.
- Обычная проверка: `cargo build --release` (с `ZAPRET_GUI_REPO=20XMAS24/zapret-gui`, иначе берётся тот же дефолт).
- Тесты: `ZAPRET_NO_MANIFEST=1 cargo test` (иначе тестовый exe требует прав администратора).
- Без админа и без single-instance: `ZapretGUI.exe --preview --page=N [--maximized]`.
- **Не обновлять** закреплённые в `Cargo.lock` крейты: parking_lot 0.12.3, parking_lot_core 0.9.10, native-tls 0.2.12,
  schannel 0.1.27, libloading 0.8.6, arboard 3.4.1, webbrowser 1.0.1. Новые версии тянут `windows-link`/raw-dylib,
  которым нужен `dlltool`+`as`, а их в GNU-toolchain нет. `cargo update` без `-p` ломает сборку.
- egui 0.29 (glow). Цвета палитры — функции с префиксом `c_` (`c_text()`, `c_muted()`…) в `theme.rs`,
  чтобы не конфликтовать с локальными переменными.

## Выпуск и обновления
- Выпуск — скилл `release` (`.claude/skills/release`). Коротко: `release.ps1 -Version X -Publish -NotesFile dist\notes-X.md`.
- Приложение само проверяет `releases/latest` при старте и каждые 6 ч, качает `ZapretGUI-Setup-<ver>.exe`,
  сверяет с `.sha256` и ставит тихо в папку, откуда запущено (`/DIR=`). Текст релиза = окно «Что нового».
- Версии с 2.0.0 автообновиться на 2.0.1+ не могут сами: нужно один раз нажать «Обновить ZapretGUI».
- Заменить локальный exe пользователя без релиза — скилл `install-local`.

## Грабли, на которые уже наступали
- PowerShell 5.1: `2>&1` на нативной команде под `$ErrorActionPreference="Stop"` превращает любой stderr
  (даже `Compiling ...` от cargo) в исключение. `release.ps1` запускать через `powershell -File ... *> log`.
- В Git Bash `tar` — это GNU tar, он портит пути `C:\`. В скриптах только `%SystemRoot%\System32\tar.exe`.
- Регекс поднятия версии должен менять только первую строку `version = ` (иначе зацепит windows-sys).
- TgWsProxy (Flowseal/tg-ws-proxy) работает в secure-режиме: ссылке для Telegram нужен секрет с префиксом `dd`
  (`tg::telegram_link`), в config.json хранится 32 hex без префикса. Исправлено в 2.0.2.
- TgWsProxy — PyInstaller onefile, поэтому в списке процессов всегда два `TgWsProxy_windows` (родитель+дочерний), это норма.
- Перед заменой `ZapretGUI.exe` пользователь должен сам закрыть программу (она держит WFP-правила и TUN);
  не убивать её по заголовку окна — так однажды был убит не тот процесс.

## Не проверено на реальной машине
Реальный туннель/kill switch с живым прокси, полный прогон установщика и цепочка автообновления 2.0.x → 2.0.y.

## Стиль общения
Пользователь пишет по-русски, тексты UI и релизов — по-русски. Коммиты — по-английски.
