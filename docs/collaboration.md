# Peer-to-peer collaboration (design)

Status: **planned**. Goal: several people edit the same KuArt document at the same time, with
live cursors, selections and comments, **without an account or a KuArt server**.

## 1. Transport: WebRTC data channels

* Each KuArt app is a peer. Peers connect directly with WebRTC (DTLS-encrypted data
  channels); STUN finds public addresses, and TURN can be configured for strict networks.
* **Signaling without a server:** the host shows an *invite code* (the WebRTC offer, compressed
  and base64-encoded, or a QR code). The guest pastes it and returns an *answer code*. Any channel
  works: chat, email, in person.
* **Optional relay:** a tiny stateless signaling relay (WebSocket, ~100 lines) can be run by a
  team so invite links connect automatically. It never sees document content.
* Rust stack: [`str0m`](https://github.com/algesten/str0m) (sans-IO WebRTC, no async runtime
  required) on a background thread, waking the UI through the existing event-loop proxy.
* Sessions are a full mesh for small teams (≤ 8 peers); larger teams use the host as hub.

## 2. Document sync

KuArt edits are already small reversible operations (`Op::Attach`, `Op::Detach`, `Op::Set`)
grouped into transactions. Collaboration broadcasts transactions; the work is making
concurrent ones converge.

| Problem | Design |
|---|---|
| Node ids differ per peer (arena indices) | Every node gets a stable **global id**: `(peer id, counter)`; ops on the wire reference global ids |
| Two peers set the same property | **Last-writer-wins per (node, property)** using a Lamport clock with peer id as tie-break |
| Two peers reorder or reparent | Children positions use **fractional indices** (as Figma does), so moves never conflict |
| Delete vs. edit of the same node | Deletes are tombstones; edits to a tombstoned node are kept but hidden (undo restores) |
| Undo | Undo is per peer: it inverts *your* last transaction, re-sent as a new transaction |
| Joining late | The host sends a `.krt` snapshot (already compact and deterministic) plus the op log since |
| Offline edits | Ops queue locally and are merged on reconnect with the same rules |

Text content uses a sequence CRDT for character-level merging once on-canvas text editing
exists; until then text is a property (last writer wins).

## 3. Presence and comments

* Presence messages (unreliable channel, ~20 Hz): cursor position, current page, selection,
  viewport, name and colour. Drawn as named cursors and coloured selection outlines.
* "Follow" mode copies a peer's viewport.
* Comments (already in the model and `.krt`) sync as their own small ops: add, reply, resolve,
  delete, keyed by `(peer id, counter)`.

## 4. Security and privacy

* Data channels are end-to-end encrypted (DTLS); the optional relay only forwards offers.
* Invite codes contain a one-time session secret; peers authenticate each other with it, so a
  leaked relay address does not let strangers join.
* Roles: host decides *edit*, *comment* or *view* per peer; viewers' ops are rejected.

## 5. Milestones

1. Global ids and fractional child ordering in the model and `.krt` (no networking).
2. Op log with Lamport clocks; deterministic merge tests (randomised two-peer and three-peer
   histories converge to identical `.krt` bytes).
3. WebRTC session with manual invite codes; snapshot + live ops between two peers.
4. Presence: cursors, selections, follow.
5. Comment sync, roles, optional relay, reconnect and offline queue.
