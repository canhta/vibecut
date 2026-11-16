# Issue tracker

Tickets are GitHub issues on this repository: the single source of truth. Workers read and edit only their assigned issue and re-read it before commenting. `gh issue list` with the labels below lists ready work.

| Ticket fact | Where it lives |
|---|---|
| Status | Open or closed. `blocked` label while a blocker is open or a decision is pending. Closed as completed is resolved; closed as not planned is wontfix, with its reason in a comment |
| Type | `enhancement` (task), `research`, `bug` |
| Priority | `P0` to `P4` label |
| Assignee | A comment naming the worker model id when claimed, for example `claude-haiku-5-5` or `claude-sonnet-5-5`; `human` for the maintainer |
| Blocked by | A `**Blocked by:**` line in the body listing issue numbers only; put any condition or note in the body text |
| Scope and acceptance | The body: Problem, Decisions, Must, Ownership, Verify |
| Worker handback | A comment on the issue ([orchestration](orchestration.md#worker-loop)) |
| Review report | A comment on the issue or the pull request |

Claimed means a live worker holds the ticket. A queued but ready ticket is open with no claim comment; one that needs intervention carries `blocked` with the condition in the body. Keep a broad parent open while a leaf is active and close it only after all its acceptance. Record unrun checks as unverified.

Record the delivery stage in a comment: **merged unbuilt** (main SHA and what the combined build must check), then close once the combined build is green.

Priority follows the [delivery order](../product/requirements.md#scope): P0 foundation, P1 core editor, P2 AI editing and agent control, P3 Home, P4 long video to shorts; a defect takes the priority of the feature it breaks. A ticket is unblocked when every blocker is closed as completed. Put the outcome of a research ticket in a comment headed `## Answer`.

Ready order: highest priority, then most open tickets transitively blocked, then lowest number. Claiming, resolving and routing follow [orchestration](orchestration.md).

Local experiments, reference captures and sample media are not in the repository. Keep them in git-ignored `.spikes/` and `.samples/` in your own checkout; a ticket that needs one says where the worker gets it.
