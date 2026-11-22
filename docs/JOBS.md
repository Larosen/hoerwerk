# Audiobook jobs: states, transitions, restart

This file is the specification of the job life cycle (implemented in `rust/hoerwerk/src/pipeline.rs` and
`engine.rs`). Any transition not listed here is a bug.

## Terms

A **job** is an audiobook on its way from the inbox (or from the library) into the library. It has:

- **`status`**: what the interface shows.
- **`stage`**: the step that runs next or is running now.

| stage | queue | status while working | what it does |
|---|---|---|---|
| `prepare` | `io` | `matching` | check files, collect hints, look for matches (shop, content, requests) |
| `convert` | `encode` | `converting` | build the M4B with chapters (or take it over) |
| `finish` | `io` | `tagging` | check for duplicates, write tags and cover, file it, clear the source away |

**Final states:**

| status | meaning |
|---|---|
| `done` | Filed. `dest` is the M4B in the library. |
| `review` | Waiting for a decision: no match or an uncertain one, or a duplicate. |
| `failed` | Error, `error` says why. Can be retried. |
| `cancelled` | Cancelled or skipped. |

`queued` means: the step `stage` is waiting for a free slot in its queue.

## Transitions

```text
                ┌──────────── retry / reopen / choose / manual ────────────┐
                ▼                                                          │
 (new) ──► queued[prepare] ──► matching ──► queued[convert] ──► converting ─┤
                                  │                                  │      │
                                  │ volume already there             │ no match
                                  ▼                                  ▼      │
                              cancelled                     review[finish] ─┘
                                                                     │ choose / manual
                                  queued[finish] ◄───────────────────┘
                                        │
                                        ▼
                                     tagging ──► done
                                        │
                                        ├─► review   (duplicate, setting "ask")
                                        └─► cancelled (duplicate, setting "skip")

 any working step ──► failed      (error)
 any working step ──► cancelled   (cancelled by the user)
 queued / review  ──► cancelled   (cancelled while nothing is running)
```

| from | to | trigger | condition / effect |
|---|---|---|---|
| – | `queued` (prepare) | new folder in the inbox, upload, "Convert" from the library | folder is complete (no change for `SETTLE_SECONDS`) |
| `queued` | `matching` / `converting` / `tagging` | a worker takes the step | **Only one worker per job.** The change happens under a lock and only from `queued`. |
| `matching` | `queued` (convert) | match found or none needed | – |
| `matching` | `cancelled` | this volume of a series is already in the library | – |
| `converting` | `queued` (finish) | M4B ready, match known | – |
| `converting` | `review` (finish) | M4B ready, no match | – |
| `tagging` | `done` | filed | `dest` set, source moved to `_fertig` or deleted, work folder gone |
| `tagging` | `review` | duplicate and `ab_duplicates=ask` | `duplicate` is set |
| `tagging` | `cancelled` | duplicate and `ab_duplicates=skip` | source cleared away |
| `review` | `queued` (finish) | match picked (`choose`) or own details (`manual`) | – |
| `review` | `queued` | duplicate decided (`replace`/`keep`) | – |
| `failed`, `review` | `queued` | `retry` | stage `finish` if the M4B still exists, otherwise `prepare` |
| `done` | `review` (finish) | `reopen` (fetch back from the library) | – |
| any working step | `failed` | exception | `error` set, work folder kept until the retry |
| any working step | `cancelled` | user cancels | ffmpeg is stopped at once, work folder gone |
| `queued`, `review` | `cancelled` | user cancels | waiting step removed from the queue |
| `done`, `failed`, `cancelled` | (removed) | "Remove finished" or delete | – |

## Restart and shutdown

| state at shutdown | after the start |
|---|---|
| `queued`, `matching`, `converting`, `tagging` | `queued` in the same `stage`: the step runs again. A finished M4B in the work folder is reused. |
| filing interrupted (filing journal `filing` set) | If the M4B is complete in the library (name and size match), the job is `done` and the old edition is archived or deleted. Otherwise the old edition that was set aside comes back and the job is `queued[finish]`. Half copies (`.hw-part-*`) are removed. |
| `docker stop` (SIGTERM) | A running ffmpeg ends at once. The job is `queued` (not `cancelled`) with the message "interrupted on shutdown". |
| final states | unchanged |

**Start:** no job is worked on before the library has been read completely once (duplicate check).
If it cannot be read, Hörwerk waits and tries again every minute.

## Guarantees (tested)

- A job never runs in two workers at once, even if it is queued several times.
  Test: `engine.rs` `one_step_per_job_at_a_time`.
- An M4B only appears in the library complete (atomic move, also across disks).
  Test: `filing.rs` `moves_whole_files_also_across_disks`.
- A restart loses no job and files none twice; shutdown puts running jobs back.
  Tests: `contract/test_contract.py` `Lifecycle.test_interrupted_job_is_finished_after_a_restart` and
  `test_sigterm_ends_in_order`.
- A cancel reaches ffmpeg at any moment, also during start-up, child processes included.
  Test: `encoder.rs` `runner_kills_a_cancelled_process`.

## Persistence

A job is one row in the table `jobs` (see `contract/schema.sql`):

- columns `id`, `kind='audiobook'`, `status`, `source`, `created`, `updated`;
- column `data` with all fields as JSON.

Known fields in `data` (`struct Job` in `rust/hoerwerk/src/pipeline.rs`):

| group | fields |
|---|---|
| origin | `id`, `source`, `rel`, `name`, `origin` |
| state | `status`, `stage`, `message`, `progress`, `log`, `error`, `created`, `updated` |
| files | `files`, `duration`, `m4b`, `cover`, `dest`, `converted_info` |
| matching | `hints`, `candidates`, `match`, `region` |
| duplicate | `duplicate`, `dup_decision` |
| chapters | `chapter_names`, `rename_files` |
| other | `pack`, `filing` |

Unknown fields are ignored when reading, so older and newer databases stay readable.
