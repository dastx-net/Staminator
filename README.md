# Staminator

Staminator is a touch typing tutor written in Rust. It is inspired by [Stamina](https://stamina.ru/), the keyboard trainer created by Alexey Kazantsev. Our first goal is to bring its familiar training experience to a modern application; improvements can follow.

## Thank you, Stamina

Stamina was part of a happy childhood. Thank you, Alexey, for that and for all the hours saved since then by knowing how to type without looking at the keyboard.

We like Stamina's approach to learning: mistakes count, but they are not punished harshly. You can keep practicing instead of worrying about every wrong key. Your fingers will get there, even if they occasionally take the scenic route.

## Why a rewrite?

We wanted Russian text to display correctly on modern systems. So, naturally, we decided to rewrite the whole thing in Rust.

This might sound excessive, but we have AI agents now, and they need typing practice too.

## Current status

Staminator is a work in progress for Windows. It has lessons in ten languages, five practice modes, user profiles, results, a lesson editor, and a separate on-screen keyboard. It also supports practice with an external UTF-8 text file.

The application does not yet match every feature or behavior of the original. See the [task plans](docs/plan/README.md) for progress and [research notes](docs/research.md) for details about Stamina.

## Languages and default lessons

The interface and lessons support the following languages. Each includes a default course adapted from KDE KTouch:

| Language | Lessons |
| --- | ---: |
| English | 15 |
| Russian | 16 |
| Spanish | 15 |
| Arabic | 13 |
| French | 28 |
| Portuguese (Brazil) | 11 |
| German | 50 |
| Italian | 60 |
| Turkish | 15 |
| Persian | 13 |

The lesson language, interface language, and keyboard layout are selected independently. On-screen keyboard layouts are currently available for Russian and English; use a system layout for the other languages. Phrase content is not included for the other eight languages. See the [lesson documentation](docs/lessons.md) and [course sources and licensing](lessons/README.md) for details.

## Run from source

On Windows, install [Rustup](https://rustup.rs/) and the Visual Studio C++ build tools. Then run this command from the repository root:

```powershell
cargo run -p staminator-desktop
```

Rustup will use the toolchain pinned in `rust-toolchain.toml` and install it if needed. An installer is planned for the future; for now, building from source is the way to start.

Choose a mode and keyboard layout in the Mode menu. Press Space to start, Esc to pause, and Space to resume. Press Esc again or Ctrl+C to finish and see the result. The on-screen keyboard can be moved to another monitor or hidden in Options.

The optional local `src/stamina_setup` directory can supply original data and resources; it is not included in the repository. Without it, the application uses built-in exercises and a neutral background. User data is stored separately in `%APPDATA%\Staminator\staminator.sqlite3`. External practice texts must use UTF-8.

## Development

Run the workspace tests and check text file encoding with:

```powershell
cargo test --workspace
./scripts/check-utf8.ps1
```

The original Stamina source is kept separately as local reference material at `../Stamina` on the project's development machine. It is not included in this repository and may not be available on other machines. The optional local installation is excluded from Git and may contain personal data and third-party media.

## License and credits

The new Staminator implementation and its documentation are available under the [MIT License](LICENSE), within the rights of their authors. This does not cover the original Stamina source, its resources, or other third-party materials. Their rights, and the right to distribute this implementation, require separate review; see the [licensing decision](docs/adr/0003-license.md).
