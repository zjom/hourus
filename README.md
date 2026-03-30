# hourus

A command-line time-tracking tool that parses `.hours` log files and generates time summaries.

## Usage

```
hourus [OPTIONS] [COMMAND]
```

**Options:**
- `--path <PATH>` — Path to `.hours` log file (reads from stdin if omitted). Defaults to HOURUS_DEFAULT_FILE env var..
- `--from <DATE>` — Filter entries starting from this date
- `--to <DATE>` — Filter entries up to this date
- `--no-env` — Do not use the HOURUS_DEFAULT_FILE env as file path

**Commands:**
- _(default)_ — Print total hours worked
- `breakdown` — Print hours broken down by task, sorted by duration
- `start <DESCRIPTION>` — Start a new session (ends any open session automatically)
- `end` — End the current open session

### Examples

```sh
# Total hours from a file
hourus --path work.hours

# Breakdown by task for a date range
hourus --path work.hours breakdown --from 2025-01-01 --to 2025-01-31

# Start and end sessions
hourus --path work.hours start "code review"
hourus --path work.hours end

# Pipe from stdin
cat work.hours | hourus
```

## Log File Format

Each line follows this structure:

```
[KIND] - [DATETIME] - [DESCRIPTION]
```

- **KIND**: `START` or `END` (case-insensitive)
- **DATETIME**: `YYYY-MM-DD HH:MM:SS` or `YYYY-MM-DDTHH:MM:SS`
- **DESCRIPTION**: Task name (stored lowercase)

**Example:**

```
START - 2025-01-15 09:00:00 - feature work
END - 2025-01-15 11:30:00 - feature work
START - 2025-01-15T13:00:00 - code review
END - 2025-01-15T14:15:00 - code review
```

## Installation

Requires Rust 1.92+.

```sh
cargo build --release
```

The binary will be at `target/release/hourus`.

## Development

```sh
cargo test
```
