# Closed command catalog

Status: Proposed. It takes effect when one declaration replaces the split matches; until then the current contract stays in the [engine README](../../crates/engine/README.md#control).

The command set is one closed enum, not a string registry. Each declaration carries every per-command fact once: its execution family, its availability rule, its grant class, whether a curated transport tool exposes it, and its lifecycle routing. Executors live per family under `session/edit/` and are reached through the declaration. Schema, digest and the agent-facing [command reference](../product/commands.md) are generated from that one source by one command.

The reason is failure direction. A string registry ends in wildcard arms that answer "enabled" for a renamed id and "denied" for a new one; adding one command touched fifteen files and needed a follow-up digest commit. A closed declaration fails to compile when a fact is missing, so a renamed command cannot stay silently reachable and a new command cannot be silently denied. The digest and schemas stay stable for idempotent control callers because they derive from the same declaration.

Rejected: per-transport allowlists (three hand-maintained lists drift), an open command registry (unknown ids become runtime policy), and deriving availability from the enabled state of a running Session (a catalog read would need a Session).

Current contracts live in the [engine README](../../crates/engine/README.md#control-admission-revisions-and-receipts); [structure](../engineering/structure.md#engine-cratesenginesrc) owns where the families and executors sit.
