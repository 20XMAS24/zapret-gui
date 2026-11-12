---
name: release
description: Publish a new ZapretGUI version to GitHub Releases (20XMAS24/zapret-gui) so installed copies auto-update. Use when the user says "выпускай X.Y.Z", "сделай релиз", "выпусти обновление", or asks to ship a fix to colleagues.
---

# Выпуск новой версии ZapretGUI

Рабочая папка: `D:\zapret_GUI\ZapretGUI-rs`. Нужны: Rust GNU, Inno Setup 6, `gh` (уже авторизован).

## 1. Подготовка
1. `git status` — все изменения в `src/` закоммичены (сообщение на английском, с Co-Authored-By из системной подсказки).
2. `cargo build --release` проходит без ошибок и предупреждений (PATH — см. `CLAUDE.md`).
3. Новая версия — следующая patch (`2.0.2` → `2.0.3`), если пользователь не назвал другую.
   Текущая — строка `version` в `Cargo.toml`, опубликованные — `gh release list --repo 20XMAS24/zapret-gui`.

## 2. Текст релиза
Создать `dist\notes-<ver>.md` по-русски. Его увидят пользователи в окне «Что нового», поэтому
писать для людей, а не про код. Формат, который понимает приложение (`render_notes` в `ui.rs`):

```markdown
## Новое
- ...
## Исправлено
- ...
## Важно
Что пользователю сделать руками (если нужно).
```

Только `##`-заголовки, пункты `- ` и обычные строки; `**` и обратные кавычки вырезаются.
Строку про установщик скрипт допишет сам.

## 3. Сборка и публикация
Запускать **отдельным процессом** (в PowerShell 5.1 перехват `2>&1` ломает скрипт на stderr cargo):

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\release.ps1 -Version <ver> -Publish -NotesFile dist\notes-<ver>.md *> dist\release-<ver>.log
"exit=$LASTEXITCODE"; Get-Content dist\release-<ver>.log -Tail 15
```

Скрипт: поднимает версию в `Cargo.toml`, собирает exe с `ZAPRET_GUI_REPO`, качает официальный zapret,
TgWsProxy и sing-box 1.14.4 в `dist\stage` (лимит 300 МБ), собирает `dist\ZapretGUI-Setup-<ver>.exe`
+ `.sha256` и делает `gh release create v<ver>`. Если релиз с такой версией уже есть — падает, нужна новая версия.
Длится 2–5 минут; ставить timeout 600000.

## 4. Проверка и коммит
```bash
git add Cargo.toml Cargo.lock && git commit -m "Release <ver>" && git push origin HEAD
gh api repos/20XMAS24/zapret-gui/releases/latest --jq '.tag_name, (.assets[]|.name+" "+(.size|tostring))'
curl -sL https://github.com/20XMAS24/zapret-gui/releases/download/v<ver>/ZapretGUI-Setup-<ver>.exe.sha256
```
`latest` должен быть `v<ver>`, два файла, хэш совпадает с выводом скрипта.

## 5. Ответ пользователю
Ссылка на релиз; что установленные копии обновятся сами (при запуске или в течение 6 ч, либо кнопкой
«Обновить ZapretGUI»); копии на 2.0.0 — только кнопкой. Если нужно, предложить скилл `install-local`,
чтобы сразу обновить `D:\zapret_GUI\ZapretGUI.exe`.
