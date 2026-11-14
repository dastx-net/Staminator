# Автоматизация issue

Бот берёт issue из GitHub и доводит до pull request через Codex. Работает на домашнем сервере easy-day под отдельной учёткой `stamibot` (без sudo, docker и libvirt).

## Когда бот берёт issue

- Issue открыт в `dastx-net/Staminator` и назначен на `losinkatorf`.
- Назначил доверенный инициатор: `DanielSanIT` или сам `losinkatorf` (список `initiators`).
- Расход лимита Codex меньше 75%. Проверка идёт перед каждым ходом Codex. Если лимит выше, бот пишет в issue и ждёт сброса.

Issue, назначенный кем-то другим, бот игнорирует. Комментарии посторонних в промпты не попадают.

## Стадии

| Стадия | Кто | Что делает |
| --- | --- | --- |
| `plan` | `gpt-6-sol` | Пишет план в `.stamibot/plan.md` |
| `plan_review` | `gpt-6-astra` | Ревью плана. P0/P1 возвращают план на доработку |
| `implement` | `gpt-6-sol` | Реализация и локальные проверки. Коммит делает бот |
| `impl_review` | `gpt-6-astra` | Ревью диффа. P0/P1 возвращают на доработку |
| `ci_push`, `ci_wait` | бот, Jenkins | Push ветки, ожидание результата сборки |
| `pr` | бот | Открывает pull request, пишет ссылку в issue |
| `pr_open` | бот | Следит за pull request и issue, ждёт замечаний |
| `done` | бот | Pull request смержен или закрыт. Ещё 14 дней (`followup_days`) бот следит за новыми комментариями |

- На любой стадии Codex может задать вопросы. Бот публикует их комментарием и ждёт ответа инициатора в issue.
- Лимит кругов ревью и починок CI: 3. Дальше бот останавливается (`blocked`) и пишет причину. Продолжить: комментарий `/retry` с необязательными указаниями.
- Упавший CI возвращает задачу на `implement` с хвостом лога.
- Закрытие issue или снятие назначения до открытия pull request отменяет работу.

## Замечания после открытия pull request

Учитываются замечания доверенных инициаторов: ревью с текстом (`Request changes` или `Comment`), построчные комментарии, комментарии в pull request и новые комментарии в issue.

- Pull request открыт: новый круг `implement`, `impl_review`, CI, пуш в ту же ветку. Бот отвечает в pull request, что сделано. На вопрос отвечает текстом без правок кода.
- Pull request смержен или закрыт (`done`): бот создаёт ветку `issue-N-followup-K` от `master`, проходит тот же круг и открывает новый pull request.
- Вопросы Codex на этих кругах бот задаёт в pull request, если он открыт, иначе в issue.
- Комментарий, пришедший во время работы, не прерывает ход Codex: бот подберёт его, когда вернётся к слежению.
- Комментарии бота помечены `<!-- stamibot -->`: так он отличает их от ответов человека (пишет от имени `losinkatorf`).

## Как бот узнаёт о событиях

Опрос GitHub API раз в минуту: `stamibot.timer` запускает `bot.py tick`. Сервер стоит за NAT, вебхуки не нужны. Один запрос в минуту на список issue, по одному на каждый открытый pull request бота и по два на каждый недавно завершённый issue. Комментарии читаются только при изменившемся `updated_at`. Issue обрабатываются по очереди.

## Размещение на easy-day

| Путь | Что |
| --- | --- |
| `~stamibot/stamibot/bot` | Клон репозитория с кодом бота (ветка `master`) |
| `~stamibot/stamibot/repo`, `wt/issue-N` | Рабочий клон и worktree на каждый issue |
| `~stamibot/stamibot/state/issue-N` | Состояние и логи ходов Codex |
| `~stamibot/.config/stamibot/gh-token` | Токен GitHub, режим 600 |
| `~stamibot/.config/stamibot/config.json` | Переопределения настроек из `DEFAULTS` в `bot.py` |
| `/srv/data/staminator-ci` | Результаты CI (`results/<sha>.json`), сборки Windows (`artifacts/<sha>`), кеш cargo |

Сеть: всё идёт через прокси A&D `127.0.0.1:18888`, кроме GitHub, OpenAI и crates.io (как в devuw).

Команды под `stamibot`:

```bash
python3 ~/stamibot/bot/automation/issuebot/bot.py status   # стадии issue и расход лимита
journalctl --user -u stamibot.service -f                   # журнал
systemctl --user stop stamibot.timer                       # остановить опрос
git -C ~/stamibot/bot pull                                 # обновить код бота
```

## CI в Jenkins

Задача `staminator` на Jenkins easy-day (`http://192.168.1.98:8080`) опрашивает GitHub раз в минуту и собирает каждую новую ревизию любой ветки по `Jenkinsfile`:

1. Собирает образ `staminator-ci` из `automation/ci/Dockerfile` (версия Rust из `rust-toolchain.toml`).
2. Запускает `automation/ci/run.sh`: UTF-8, fmt, clippy, тесты с покрытием 95%, сборка `x86_64-pc-windows-gnu`.
3. `automation/ci/report.py` кладёт результат в `/srv/data/staminator-ci/results/<sha>.json`. Бот читает этот файл: учётка Jenkins ему не нужна.

## Ограничения

- End-to-end в Windows VM (`ad-teams-win11`) не подключён: в гостевой системе нет SSH, доступен только RDP.
- Одобрение (`Approve`) и ревью без текста бот не обрабатывает. Мержит человек.
- Codex и токен GitHub работают под одной учёткой. Изоляция держится на отдельном пользователе и списке доверенных инициаторов.
- Каталог `/srv/data/staminator-ci` открыт на запись всем локальным пользователям (на сервере нет `setfacl`).
