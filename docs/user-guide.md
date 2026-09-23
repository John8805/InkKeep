# InkKeep User Guide

English | [繁體中文](user-guide.zh-TW.md)

## Opening the window

Run the executable, press `Alt+.` (configurable), or choose "Show window" from the system tray icon.

The hotkey only opens the window; pressing it again keeps the window open. Each time it opens, the window is centered on the monitor you are working on.

`Esc` or the ✕ in the title bar hides the window, and InkKeep keeps running in the system tray. To quit, choose "Quit" from the tray menu.

The window is a normal Windows window: it has a title bar, can be minimized and maximized, shows up in the taskbar, and does not stay on top.

Running the executable again while InkKeep is running does not start a second copy. If the vault is open, you get a notification that InkKeep is running in the background; otherwise the window opens.

## Two-tier keys

The master password goes through Argon2id twice with different salts, producing two keys that cannot be derived from each other.

| | Tier 1 | Tier 2 |
|---|---|---|
| Protects | The whole KDBX file: snippets, bookmarks, titles, usernames | Only the password field of password entries |
| Stored in | Windows Credential Manager | Nowhere; derived again when needed |
| Asks for the master password | The first time on a new computer | The first time you **read** a password after an idle lock |

Day to day, InkKeep opens straight to search, and snippets and bookmarks are ready to use.

Tier 2 is a public/private key pair. The public key is stored in the vault, so adding or changing a password needs no master password. The private key is derived from the master password each time and never stored; you need it only to read a password back:

| Action | Master password needed? |
|---|---|
| Add or change a password | No |
| Change a title, username or tags | No |
| Send a password (Enter) or copy it | Yes |
| Press "Show" in the editor | Yes |
| Send or copy a username or site | No; they are not encrypted |

When you edit an existing password entry, the password field is empty. Leave it empty to keep the current password.

After you enter the master password once, InkKeep does not ask again until the idle lock.

The public key can only encrypt: someone holding the tier 1 key can add or overwrite password entries, but cannot read the existing ones.

On a new computer you only need your master password. Enter it once and the key is saved in that computer's Credential Manager.

The tier 1 key in Credential Manager is protected by DPAPI with your Windows sign-in: another Windows account cannot read it, and neither can another machine with your disk. To turn this cache off, enable "Ask for master password at startup" in Settings.

**Reading password entries always requires the master password.** Snippets and bookmarks can be read with the key in Credential Manager.

## Search

| Key | Action |
|---|---|
| Typing | Live search. `#tag` filters by tag |
| `Tab` / `Shift+Tab` | Switch type: snippets → bookmarks → passwords |
| `↑` `↓` | Select |
| `PageUp` / `PageDown` | Move 10 entries |
| `Enter` | Send. For password entries with a username, choose username or password first |
| `Shift+Enter` | Copy to the clipboard only |
| `Ctrl+Shift+Enter` | Send once with the other input method |
| `Ctrl+N` / `Ctrl+E` | New / edit |
| `Ctrl+,` | Settings |
| `Ctrl+L` | Lock passwords (snippets and bookmarks stay available) |
| `Esc` | Close the window |

Snippet and bookmark rows have copy and edit buttons on the right. For password entries, the preview pane has a copy button next to the username, site and password.

Search splits the query on spaces, and every token must appear in the title or content. Exact title matches come first; the rest are sorted by how often you use them. Password entries match on title, username and site; the password itself is never searched.

## Template placeholders

Only snippets are expanded.

| Placeholder | Meaning | Example output |
|---|---|---|
| `${date}` | Today, ISO format | `2026-09-23` |
| `${date:+7d}` | Offset; units `d` `w` `m` `y` | `2026-09-30` |
| `${date::long}` | Format: `iso` (default), `long`, `short`, or strftime | `2026年9月23日` |
| `${date:+7d:long}` | Offset and format together | `2026年9月30日` |
| `${time}` / `${time:12h}` | Time | `18:34` / `下午6:34` |
| `${datetime}` | Date and time | `2026-09-23 18:34` |
| `${clipboard}` | Current clipboard text | |
| `${cursor}` | Where the cursor ends up after sending | |
| `${input:label}` | Ask in a form before sending | |
| `${input:label:default}` | With a default value | |
| `${select:label:a\|b\|c}` | Drop-down choice | |
| `${uuid}` | Random UUID v4 | |
| `${snippet:other title}` | Insert another snippet, up to 5 levels deep | |

`$${` produces a literal `${`. Put a backslash before `:` `|` `}` inside arguments.

Braces in snippets are kept as they are, for example `fn main() { }`; only `${` starts a placeholder.

Month and year offsets follow the calendar and clamp to the last day of the month (January 31 plus one month is February 28).

## How content is sent

| Type | Method |
|---|---|
| Snippets, bookmarks | Write to the clipboard, then `Ctrl+V`; the previous clipboard text is restored afterwards |
| Passwords, usernames | Typed character by character with `SendInput`, without the clipboard |

Password entries are always typed, whether you send the username or the password. The clipboard is shared by the whole system, and background programs and clipboard history tools can read it, so credentials stay off it.

The clipboard is restored 0.3 seconds after pasting, so the target app has time to read it first. If you copy something else in the meantime, InkKeep keeps your copy. If the clipboard held an image or files, it is empty after the restore.

When snippets and bookmarks go through the clipboard, InkKeep sets three exclusion markers that keep them out of Windows clipboard history (`Win+V`) and cloud clipboard. Third-party clipboard managers can still see them.

While typing a password, InkKeep checks the foreground window every 32 characters and stops at once if you switch away.

## Workspaces

Snippets, bookmarks and passwords can be grouped into workspaces, such as "Personal" and "Work". Pick a workspace from the drop-down left of the search box, or choose "All" to search everything. Workspaces have no hotkey.

- Every entry belongs to one workspace. The first time a vault is opened, InkKeep creates "Personal" and puts all existing entries in it
- Add, rename and delete workspaces in the Workspaces section of Settings; only empty workspaces can be deleted
- New entries go into the workspace selected in search; you can change it in the editor
- Bookmark and password imports ask for a target workspace. Duplicates are checked only within that workspace,
  so the same URL can be in both "Personal" and "Work"

Each workspace is a group in the KDBX file, so other KDBX tools show the same folders.

## Importing bookmarks

The source list in Settings → Bookmark import shows every browser profile on the computer by the browser's own name,
such as "Chrome — John" and "Chrome — Work". Each profile can be imported into a different workspace.

Chrome and Edge profile names come from `User Data\Local State`, Firefox names from `profiles.ini`.
Guest profiles are not listed. You can import from Firefox while it is running, including bookmarks you just added.

## Importing passwords

Settings → Password import → choose the target workspace → choose a CSV exported from another password manager → review the counts → import.

InkKeep reads the column names in the header row, so it is not tied to one source. It recognizes exports from Chrome, Edge, Firefox,
Bitwarden, 1Password, KeePassXC and LastPass, and other CSVs with matching column names.

- No master password is needed; passwords are encrypted with the vault's public key
- Folders (Bitwarden folders, KeePassXC groups) become tags
- Entries whose site and username already exist in the same workspace are skipped
- Rows with an empty password are skipped
- Notes are not imported, because password entries have no notes field; the preview shows how many notes were left out

**Exported CSV files are plain text.** After the import, Settings offers "Delete this CSV file"; please use it,
and keep the CSV out of folders that sync to the cloud.

For browser passwords, use the browser's own "Export passwords" feature. InkKeep does not read the browser's password store.

## Settings in the file only

The timing values used when sending are not in the Settings screen; you only need them when troubleshooting an app that does not receive input.
Edit `[inject]` in `settings.toml`:

```toml
[inject]
focus_timeout_ms = 300      # How long to wait for focus to return to the target window
paste_settle_ms = 30        # Wait after pasting before moving the cursor
type_delay_ms = 2           # Delay between typed characters
restore_delay_ms = 300      # Wait after pasting before restoring the clipboard
virtual_input = false       # Type passwords with scan codes, like a physical keyboard
```

The file is at `%APPDATA%\app.inkkeep\settings.toml`. Out-of-range values are clamped to a sensible range,
and InkKeep starts normally. Restart InkKeep after editing.

To try `virtual_input` once without editing the file, press `Ctrl+Shift+Enter` to send with the other method.

## Known limitations

- InkKeep cannot send input to windows running as administrator. This is Windows UIPI: a lower-privilege program cannot send input to a higher-privilege window. InkKeep notifies you; snippets and bookmarks stay on the clipboard for you to paste, passwords do not.
- With a Chinese IME active, virtual input mode does not work, because the IME consumes scan code events. The default Unicode mode is unaffected.
- Local malware is out of scope. A program running as you can read InkKeep's memory and log keystrokes, as with any password manager. InkKeep protects against leaks through cloud sync, a lost laptop, and someone briefly using your computer.
- Anyone who can sign in to your Windows account can read snippets and bookmarks. This is the trade-off for keeping the tier 1 key in Credential Manager so InkKeep opens without a password. Password entries still need the master password. To enter it every time, enable "Ask for master password at startup".
- Clearing memory is best effort. Keys stay in memory while unlocked. `zeroize` makes sure the zeroing writes are not optimized away, but it cannot clear old copies left when a `String` reallocates or values left in CPU registers, and it cannot keep data out of swap or the hibernation file.
- Games that use DirectInput do not receive simulated input.
- Inside Remote Desktop, the local client may capture the hotkey.

## Data format

Format identifiers inside the file (`snipkit-sealed:v1:`, `snipkit.secret.*` and so on) keep the old name `snipkit`,
the name they were first written with. Existing data is recognized by these names, so they did not change with the product name.

KDBX 4.1. Each type uses native fields, so KeePassXC shows three kinds of ordinary entries:

| Type | KDBX field | What KeePassXC shows |
|---|---|---|
| Snippet | Notes | The text |
| Bookmark | URL | The URL |
| Password | Password (+ UserName, URL) | `snipkit-sealed:v1:…`; username and site in plain text |

Tags are stored in Tags and usage counts in UsageCount, both native KDBX fields. Notes and other fields you add in KeePassXC are kept when InkKeep saves.

Passwords show up as ciphertext in KeePassXC, because they have one more layer of encryption (see "Two-tier keys"). Snippets, bookmarks, titles and usernames are plain text in KeePassXC.

To open the file in KeePassXC, paste the **tier 1 key**, not the master password. "Show" under "Vault key" in Settings displays it (after you enter the master password), and the CLI command is `vault-key`. It is derived from the master password, so every computer computes the same key and you do not need to write it down.

When saving, InkKeep writes a temporary file, reads it back and compares every field, rotates the backups (3 are kept), and only then replaces the vault. If the comparison fails, the temporary file is kept, an error is reported, and the vault is left as it was. The write path of the `keepass` crate is newer than its read path, which is why this check exists.

We recommend writing only with InkKeep and using KeePassXC to read. Editing in both works, but rerun the compatibility tests when upgrading the `keepass` crate.

## Languages

Traditional Chinese and English. The default follows the system language and can be changed in Settings. To add a language, see the [development guide](development.md#adding-a-language).

## Not yet supported

- macOS, Linux
- Browser autofill (needs an extension and native messaging)
- TOTP
- Keyword triggers (type `:sig` to expand)
- Installer, auto-updater, code signing
- Merging cloud conflict copies (InkKeep detects them and shows a notice)
