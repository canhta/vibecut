# Clip edits

Interface: `execute_compound`, `execute_uncompound` and `admit_content` re-exported by `clips.rs`; `renumber` is private to `clips`.

## Compound clips

`timeline.compound {items}` moves the listed items into one new compound clip (`CreatedId::Item`) and `timeline.uncompound {items}` puts each listed compound clip back as its members, each as one undo step. The project model and the release rule belong to [project compound clips](../../../../../project/src/model/README.md#compound-clips); the decision is [ADR 0009](../../../../../../docs/adr/0009-compound-clip.md).

- One item is enough. The clip starts at the earliest member and lasts to the latest member's end; members keep their ids and relative times, and nested tracks keep the outer tracks' order, names, kinds and hidden and muted state.
- The clip goes onto the lowest video track that held a member when its span is free there; otherwise onto a new `Video` track directly above the lowest visual track that held one (above the main track for audio-only members). Its id is allocated before the nested track ids.
- A transition whose clips are all members moves inside; one that joins a member to a non-member refuses with `CompoundSplitsTransition`. Locked tracks refuse with `Locked`, an unknown id with `NoItem`, and `timeline.uncompound` on another kind with `NotCompound`.
- A compound clip edits like a video clip: trim, split (both pieces keep a copy of the nested timeline, the right one under fresh nested ids), speed, curve and reverse, fades, volume and mute, transform, opacity, blend, masks, effects and keyframes. Move, delete, hide, copy, cut, paste and duplicate act on it whole; a copy gets fresh ids for everything it nests. `timeline.insert` and `set_item_param` refuse compound content, so only `timeline.compound` allocates nested ids.
- `timeline.uncompound` refuses with `CompoundHasOwnEdits` while the clip has a window other than its whole content, a retime, a look or changed clip settings (`Item::compound_has_own_edits`): the members keep their own timing, so releasing would lose those edits, and baking them into every member is not done.
- Groups: an item in a group compounds with its whole group (`with_group_members` before the edit); the compound clip itself joins no group and its members carry none.
