# API contract

What the Hörwerk server (`rust/hoerwerk`) has to fulfil. It is tested only from the outside, over HTTP and the folders,
without importing the application. Written against Hörwerk 7.13 (then in Python): what 7.13 did is the yardstick.

| file | content |
|---|---|
| `routes.json` | All API routes with method, path and access (`public` / `user` / `admin`) – the server reads it at build time |
| `settings.json` | Settings: defaults, allowed values, environment variables, version – the server reads it at build time |
| `schema.sql` | The database schema (tables, indexes, `user_version`) |
| `fixtures/hoerwerk-7.13.db` | A database from 7.13 that every new version has to take over unchanged |
| `golden/` | The answers of 7.13 to tricky input – the Rust unit tests check against them: M4B tags of a book (`book.json` → `tags.json`, read with `read_tags.py`; read back `read.json`), names and titles (`words.json`), releases and indexers (`releases.json`), shop data and matches (`audible.json`), reviews, feeds and bestseller PDFs (`discover.json`) |
| `test_contract.py` | The tests: access, security, accounts, invitations, settings, requests, inbox → library, upload, live updates, shutdown and restart, schema, old database |
| `test_tags.py` | The tag writer `hw-tags` against `golden/` |
| `../docs/JOBS.md` | Job states, transitions and the behaviour on a restart |

```sh
(cd rust && cargo build --release)
pip install -r requirements-dev.txt
cd contract && python3 -m unittest discover -s .          # against rust/target/release/hoerwerk
HW_SERVER_CMD=/path/to/hoerwerk python3 -m unittest discover -s .
```

The golden files and the fixture are frozen. Whoever changes a rule on purpose edits the file by hand and
says why in the commit.
