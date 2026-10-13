# Dependency DAG demo

```sh
cargo run -- --root examples/dag-demo
```

Press `Tab` three times for Dependencies, `C` to show the completed root,
and `d` for a full-width graph. Use `j` / `k` to select tasks and scroll.

The graph uses compact lanes and continuous diagonal paths. Tasks on the same
row can run in parallel. Each node has a centered caption directly above it, styled in dim gray.
IDs and readiness appear on the right, in the same left-to-right order as the
nodes. Captions are shortened when needed to keep parallel tasks separate. The header shows the shared ID prefix
(`DAG-DEMO-*`); labels use the suffix to leave more room for task titles.
`◉` is the selected task, `R` means ready, `W` means waiting, and `!` means a
missing prerequisite. Selection colors its prerequisites and dependents.

```text
                 Shared contract
                        ◉                          01 R
                    /───│───\
                /───    │    ───\
            /───        │        ───\
  API skeleton   Migration plan      Console
        ●               ●               ●          02 R           04 R           03 R
         ───\       /───                │
          Indexed store                 │
                ●                       │          05 W
                │───\               /───
                │    ───\           │
                │        ───\   /───
            Load test     Console join
                ●               ●                  07 W                  06 W
                 ───\       /───
                   Release RC
                        ●                          08 W
                    /─── ───\
         Security review Canary rollout
                ●               ●                  09 W                  10 W
                 ───\       /───
                     Release
                        ●                          11 W

Missing dependency
!  DAG-DEMO-12  missing DAG-DEMO-99
```

The demo uses concise task titles; the terminal truncates longer captions. The three initial branches are
ordered `02`, `04`, `03` to put the parents of `05` together. The console
branch stays on its own rail beside `05`, then joins the console integration
at `06`. It never joins `05`. The release candidate joins `06` and `07`, then
forks again into the two release sign-off tasks.
