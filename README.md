# Either Slot

An atomic slot that both ends can access it at most once. If one end successfully places a data into it, the other end fails to place its data and obtains the former's data instead.

In other words, the data passed by both ends will eventually arrives at either of them, or be silently discarded if either end is dropped.

Beside the primitive implmentation, we also extend it to array slots and tuple slots, which resides in [`mod@array`] and [`mod@tuple`] module respectively.

## Examples

```rust
// Both ends attempt to send their data, but only one succeeds.
use either_slot::{either_slot, SendError};

let (a, b) = either_slot();
a.send(1).unwrap();
assert_eq!(b.send('x'), Err(SendError::Received('x', 1)));
// both ends cannot be used any longer after access.
// let _ = (a, b);
```

```rust
// If one end is dropped, the other end fails to send its data and retrives
// it back.
use either_slot::{either_slot, SendError};

let (a, b) = either_slot::<u8, _>();
drop(a);
assert_eq!(b.send(1), Err(SendError::Disconnected(1)));
```