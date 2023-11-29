# Either Slot

An atomic slot whose senders can either place their value into the slot, or retrive all the data from the slot.

Firstly we have the primary implmentation - [`either`], which have 2 senders attempting to send their own data into the slot. If one succeeds, the other will instead receive the data from the other sender alongside its own data. If one sender drops before the other sender sends, the latter will retrive back its own data only; but if the former drops after the latter, the data sent by the latter will be discarded.

Beside the primary implmentation, we also extend it to array slots and tuple slots, which resides in [`mod@array`] and [`mod@tuple`] module respectively.

## Examples

### The primary implementation

```rust
// Both ends attempt to send their data, but only one succeeds.
use either_slot::{either, SendError};

let (a, b) = either();
a.send(1).unwrap();
assert_eq!(b.send('x'), Err(SendError::Received('x', 1)));
// both ends cannot be used any longer after access.
// let _ = (a, b);
```

```rust
// If one end is dropped, the other end fails to send its data and retrives
// it back.
use either_slot::{either, SendError};

let (a, b) = either::<u8, _>();
drop(a);
assert_eq!(b.send(1), Err(SendError::Disconnected(1)));
```

### The advanced implementation

Check the documenetation in [`fn@array`] and [`fn@tuple`] to see the corresponding examples.

## License

MIT OR Apache-2.0