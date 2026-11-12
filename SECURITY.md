# Security policy

## Supported versions

The project is pre-1.0. Security fixes land on `main`; use the latest commit or
release.

| Version | Supported |
|---|---|
| `main` (0.1.x) | ✅ |

## Reporting a vulnerability

**Please do not open a public issue for security problems.**

Report privately through GitHub Security Advisories:

1. Open <https://github.com/aaryxnblondead/codegraph-desktop/security/advisories/new>
2. Describe the issue, the affected version/commit, and a reproduction if you
   can.

You can expect an acknowledgement within a few days. If the report is valid, a
fix and advisory will be published; credit is given unless you ask otherwise.

## What is in scope

- Code execution or path traversal caused by parsing hostile source files
- Reading or writing files outside the indexed project and the app data
  directory
- Command injection through git arguments or config values
- Denial of service that cannot be explained by project size (for example, a
  single small file that hangs or exhausts memory)
- Leaking the contents of indexed files to the network (the app has no network
  code; a finding that proves otherwise is very interesting)

## What is out of scope

- Incorrect analysis results on unusual code. Wrong edges and missed edges are
  bugs, not vulnerabilities — open a normal issue.
- Anything that requires a malicious local user who can already modify the
  project or the app config.

## How the app is expected to behave

- It never writes inside the project it indexes; caches and settings live under
  the OS app data directories.
- It never commits, checks out, or pushes: git access is read-only.
- It makes no network requests.
- A file that fails to parse is recorded and skipped; it does not abort the run.
