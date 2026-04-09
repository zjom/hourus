# hourus

A command-line time-tracking tool that parses `.hours` log files and generates time summaries.

## Usage

```
hourus [OPTIONS] [COMMAND]
```

**Global options:**
- `--path <PATH>` — Path to `.hours` log file. Falls back to `HOURUS_DEFAULT_FILE` env var, then stdin.
- `--no-env` — Ignore the `HOURUS_DEFAULT_FILE` environment variable.
- `--from <DATE>` — Only include entries on or after this date (`YYYY-MM-DD`).
- `--to <DATE>` — Only include entries on or before this date (`YYYY-MM-DD`).

**Commands:**
- _(default)_ — Print total hours worked.
- `breakdown [OPTIONS]` — Print hours broken down by task, sorted by duration.
- `start <DESCRIPTION>` — Start a new session (auto-ends any open session).
- `end` — End the current open session.

### Output formats

The default command and `breakdown` support a `--format` flag:

| Format | Flag | Description |
|--------|------|-------------|
| Pretty | `--format pretty` | Human-readable text (default) |
| JSON | `--format json` | Newline-delimited JSON |
| CSV | `--format csv` | Comma-separated values with header |
| TSV | `--format tsv` | Tab-separated values with header |

The `--format` flag is per-command: pass it after `breakdown` for breakdown output, or as a global flag for total output.

### Examples

```sh
# Total hours from a file
hourus --path example.hours

# Total hours as JSON
hourus --path example.hours --format json

# Breakdown by task for a date range
hourus --path example.hours breakdown --from 2025-01-01 --to 2025-01-31

# Breakdown as CSV (e.g. for import into a spreadsheet)
hourus --path example.hours breakdown --format csv

# Breakdown as TSV, piped to a pager
hourus --path example.hours breakdown --format tsv | less

# Start and end sessions
hourus --path example.hours start "code review"
hourus --path example.hours end

# Use the environment variable default instead of --path
export HOURUS_DEFAULT_FILE=~/example.hours
hourus breakdown --from 2025-01-01

# Pipe from stdin
cat example.hours | hourus
```

## Log File Format

Each line follows this structure:

```
[KIND] - [DATETIME] - [DESCRIPTION]
```

- **KIND**: `START` or `END` (case-insensitive)
- **DATETIME**: `YYYY-MM-DD HH:MM:SS` or `YYYY-MM-DDTHH:MM:SS`
- **DESCRIPTION**: Task name (normalised to lowercase)

**Example:**

```
START - 2025-01-15 09:00:00 - feature work
END - 2025-01-15 11:30:00 - feature work
START - 2025-01-15T13:00:00 - code review
END - 2025-01-15T14:15:00 - code review
```

## Installation

Requires Rust 1.92+.

Hourus has not been released on crates.io. 

To install you must first clone the repo and then run `cargo install`.
i.e.,

```sh
git clone github.com/zjom/hourus.git --head ./hourus
cargo install --path ./hourus
```

## Development

```sh
cargo test
cargo clippy
```
