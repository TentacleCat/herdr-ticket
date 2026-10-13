# Dependency DAG demo

Run the board from the repository root:

```sh
cargo run -- --root examples/dag-demo
```

Press `Tab` three times to open the Dependencies tab, then press `C` to show
the completed root ticket. Use `j` / `k` to move through the graph. Selecting a
ticket highlights its prerequisites and the tickets it unlocks; the Details
panel lists both sets by ID. Select `DAG-DEMO-08` to inspect a two-input join
with two downstream branches. Select `DAG-DEMO-12` to see how a missing
dependency is reported.

The graph uses compact vertical rails, like a terminal commit graph: a task is
a dot, a prerequisite path stays on its rail, and branch and join points are
drawn where paths split or meet. With `C` enabled, the sample reads like this
(the live view colors the selected task and its connected paths):

```text
● DAG-DEMO-01  shared task contract (closed)
├─● DAG-DEMO-02  API and persistence
├─┼─● DAG-DEMO-03  workflow console
● │ │ DAG-DEMO-04  migration plan
●─┘ │ DAG-DEMO-05  indexed task store
├─●─┘ DAG-DEMO-06  console integration
● │ DAG-DEMO-07  load test
●─┘ DAG-DEMO-08  release candidate
├─● DAG-DEMO-09  security review
● │ DAG-DEMO-10  canary rollout
●─┘ DAG-DEMO-11  validated release
● DAG-DEMO-12  missing prerequisite · MISSING
```

The missing prerequisite is intentionally absent from the rails and remains
visible as `MISSING` in the task row and details.
