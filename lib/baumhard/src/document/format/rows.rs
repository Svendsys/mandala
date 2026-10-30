// SPDX-License-Identifier: MPL-2.0

//! [`Rows`]: a row of a node's content — its sections, its links, a
//! frame's rings, the spans of a section, a ring, a path, a cap, a label
//! or a portal end — held as a way to reach each row, never as a list of
//! every row's DTO (ruling R19.1 on #236).
//!
//! **Why.** A node may hold any number of rows, so a node whose DTO held
//! them all would cost, while it is read or written, memory in
//! proportion to its row rather than to the one row being worked on.
//! On wasm32, where memory is the limit, one node holding every row of
//! its map would then cost about twice what the same rows spread over
//! many nodes do, and pass the limits a map is measured against.
//!
//! **Reading.** A row is captured as the borrowed text of its array
//! (`&RawValue`) and checked where it stands: every item is read as its
//! DTO, in the order of the text, and dropped. So a file's errors are
//! still found in the order of the text, at their paths, and a row that
//! reads holds one pointer and a length. The builder then reads the
//! items again, one at a time, as it appends each one's element
//! ([`Rows::each`]).
//!
//! **Writing.** The saver gives a row as a source over the arena
//! ([`RowSource`]): each item's DTO is made when it is reached, written
//! or built, and dropped.
//!
//! **An error inside a row.** A row is checked by a reader of its own,
//! so the reader that read the node only knows that the row did not
//! read. The row's reader leaves where and why in [`take_fault`]'s slot,
//! each row around it adds its own place, and the loader joins them into
//! the one error a load reports, at its full path and its place in the
//! text. The slot is per thread and is taken by the load that set it.

use crate::document::format::error::SaveError;
use crate::document::format::strict;
use serde::de::{self, Deserialize, DeserializeSeed, Deserializer, SeqAccess, Visitor};
use serde::ser::{self, Serialize, SerializeSeq, Serializer};
use serde_json::value::RawValue;
use std::cell::RefCell;
use std::fmt;
use std::marker::PhantomData;
use std::rc::Rc;

/// The message a row's reader stops the reader around it with: the row
/// did not read, and [`take_fault`] says where and why.
pub(crate) const ROW_FAULT: &str = "a row of this value did not read";

/// Why a walk over a row stopped before its end.
pub(crate) enum Halt {
    /// What the walk does with each item refused it.
    Item,
    /// An item did not read. The row was checked when it was captured,
    /// so this is a defect.
    Read(serde_json::Error),
    /// The saver refused an item: its value has no spelling.
    Save(SaveError),
}

/// Why [`Rows::each`] stopped: what `f` refused with, or [`Halt`]'s other
/// reasons.
pub(crate) enum RowsError<E> {
    /// `f` refused.
    Item(E),
    /// An item did not read (a defect: the row was checked).
    Read(serde_json::Error),
    /// The saver refused an item.
    Save(SaveError),
}

/// Where the rows of an arena come from when the saver writes them: each
/// item made as it is reached. See the module docs.
pub(crate) trait RowSource<'a, T> {
    /// Whether the row has no item.
    fn is_empty(&self) -> bool;
    /// Give each item, with its place in the row, to `sink`, in order;
    /// stop at the first refusal.
    fn each(&self, sink: &mut dyn FnMut(usize, T) -> Result<(), Halt>) -> Result<(), Halt>;
}

/// A row of items `T`. See the module docs.
pub(crate) struct Rows<'a, T> {
    source: Source<'a, T>,
    /// What the row holds, in the type: an item is a `T`. The source
    /// gives each one; this says so to whoever reads the types (the
    /// strictness gates read the format's types from the source).
    items: PhantomData<T>,
}

/// What a [`Rows`] holds.
enum Source<'a, T> {
    /// No item.
    Empty,
    /// The text of the array, checked.
    Text(&'a RawValue),
    /// The saver's way to the items.
    Arena(Rc<dyn RowSource<'a, T> + 'a>),
}

impl<'a, T> Rows<'a, T> {
    /// A row the saver makes from `source`.
    pub(crate) fn arena(source: Rc<dyn RowSource<'a, T> + 'a>) -> Self {
        Rows {
            source: Source::Arena(source),
            items: PhantomData,
        }
    }

    /// Whether the row has no item. O(1) for a row read from text; for
    /// the saver's, a scan of the parent's children until the first.
    pub(crate) fn is_empty(&self) -> bool {
        match &self.source {
            Source::Empty => true,
            Source::Text(raw) => is_empty_array(raw.get()),
            Source::Arena(source) => source.is_empty(),
        }
    }

    /// Give each item, with its place in the row, to `f`, in order: read
    /// from the text one at a time, or made by the saver one at a time.
    /// One item exists at a time. Stops at the first refusal.
    pub(crate) fn each<E>(&self, mut f: impl FnMut(usize, T) -> Result<(), E>) -> Result<(), RowsError<E>>
    where
        T: Deserialize<'a>,
    {
        let mut refused: Option<E> = None;
        let mut sink = |index: usize, item: T| match f(index, item) {
            Ok(()) => Ok(()),
            Err(error) => {
                refused = Some(error);
                Err(Halt::Item)
            }
        };
        let walked = match &self.source {
            Source::Empty => Ok(()),
            Source::Text(raw) => read_each(raw, &mut sink),
            Source::Arena(source) => source.each(&mut sink),
        };
        match (walked, refused) {
            (_, Some(error)) => Err(RowsError::Item(error)),
            (Ok(()), None) => Ok(()),
            (Err(Halt::Read(error)), None) => Err(RowsError::Read(error)),
            (Err(Halt::Save(error)), None) => Err(RowsError::Save(error)),
            // `f` refused, which `refused` holds.
            (Err(Halt::Item), None) => Ok(()),
        }
    }
}

impl<T> Default for Rows<'_, T> {
    fn default() -> Self {
        Rows {
            source: Source::Empty,
            items: PhantomData,
        }
    }
}

impl<T> Clone for Rows<'_, T> {
    fn clone(&self) -> Self {
        Rows {
            source: match &self.source {
                Source::Empty => Source::Empty,
                Source::Text(raw) => Source::Text(raw),
                Source::Arena(source) => Source::Arena(Rc::clone(source)),
            },
            items: PhantomData,
        }
    }
}

impl<T> fmt::Debug for Rows<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.source {
            Source::Empty => f.write_str("Rows([])"),
            Source::Text(raw) => write!(f, "Rows({})", raw.get()),
            Source::Arena(_) => f.write_str("Rows(<arena>)"),
        }
    }
}

/// Two rows are equal when both are empty, or both are the same text. A
/// row the saver makes is equal to no other: it is not text yet.
impl<T> PartialEq for Rows<'_, T> {
    fn eq(&self, other: &Self) -> bool {
        match (&self.source, &other.source) {
            (Source::Text(a), Source::Text(b)) => a.get() == b.get(),
            _ => self.is_empty() && other.is_empty(),
        }
    }
}

/// Whether `text`, the text of a JSON value, is an array with no item.
fn is_empty_array(text: &str) -> bool {
    text.strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .is_some_and(|inside| inside.trim_matches([' ', '\t', '\n', '\r']).is_empty())
}

/// Read each item of the array `raw` as a `T`, one at a time, and give
/// it to `sink`.
fn read_each<'a, T: Deserialize<'a>>(
    raw: &'a RawValue,
    sink: &mut dyn FnMut(usize, T) -> Result<(), Halt>,
) -> Result<(), Halt> {
    struct Each<'s, T> {
        sink: &'s mut dyn FnMut(usize, T) -> Result<(), Halt>,
        halt: &'s mut Option<Halt>,
    }
    impl<'de, T: Deserialize<'de>> Visitor<'de> for Each<'_, T> {
        type Value = ();
        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("an array")
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<(), A::Error> {
            let mut index = 0;
            while let Some(item) = seq.next_element::<T>()? {
                if let Err(halt) = (self.sink)(index, item) {
                    *self.halt = Some(halt);
                    return Err(de::Error::custom("stopped"));
                }
                index += 1;
            }
            Ok(())
        }
    }
    let mut halt = None;
    let mut deserializer = serde_json::Deserializer::from_str(raw.get());
    let read = strict::Objects(&mut deserializer).deserialize_seq(Each {
        sink,
        halt: &mut halt,
    });
    match (read, halt) {
        (_, Some(halt)) => Err(halt),
        (Ok(()), None) => Ok(()),
        (Err(error), None) => Err(Halt::Read(error)),
    }
}

/// An array read item by item as `T`, each dropped once read: how a row
/// is checked where it stands.
struct Checked<T>(PhantomData<T>);

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Checked<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Check<T>(PhantomData<T>);
        impl<'de, T: Deserialize<'de>> Visitor<'de> for Check<T> {
            type Value = ();
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a sequence")
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<(), A::Error> {
                while seq.next_element_seed(Item::<T>(PhantomData))?.is_some() {}
                Ok(())
            }
        }
        deserializer.deserialize_seq(Check::<T>(PhantomData))?;
        Ok(Checked(PhantomData))
    }
}

/// One item of a row, read as `T` and dropped.
struct Item<T>(PhantomData<T>);

impl<'de, T: Deserialize<'de>> DeserializeSeed<'de> for Item<T> {
    type Value = ();
    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
        T::deserialize(deserializer).map(drop)
    }
}

/// Where and why a row did not read. See the module docs.
pub(crate) struct Fault {
    /// The path inside each row, the outermost row's first: each is the
    /// path its reader was at when it stopped.
    pub steps: Vec<serde_path_to_error::Path>,
    /// What the innermost reader said.
    pub error: serde_json::Error,
    /// The innermost row's text, as its address and length: the reader's
    /// line and column count from its start.
    pub base: (usize, usize),
}

thread_local! {
    /// The fault of the row that last did not read on this thread.
    static FAULT: RefCell<Option<Fault>> = const { RefCell::new(None) };
}

/// Whether the text at `inner` lies inside the text at `outer`, both as
/// (address, length).
fn inside(inner: (usize, usize), outer: (usize, usize)) -> bool {
    inner.0 >= outer.0 && inner.0 + inner.1 <= outer.0 + outer.1
}

/// The address and length of `text`.
fn span_of(text: &str) -> (usize, usize) {
    (text.as_ptr() as usize, text.len())
}

/// Forget any fault left on this thread: a load starts clean.
pub(crate) fn clear_fault() {
    FAULT.with(|slot| slot.borrow_mut().take());
}

/// The fault of a row inside `extent`, when `message` — what the reader
/// of `extent` said — is a row's: taken from this thread's slot.
pub(crate) fn take_fault(message: &str, extent: &str) -> Option<Fault> {
    let fault = FAULT.with(|slot| slot.borrow_mut().take())?;
    (message.starts_with(ROW_FAULT) && inside(fault.base, span_of(extent))).then_some(fault)
}

/// Check the row `raw` where it stands: read every item as a `T`, in
/// order. A row that does not read leaves its fault in the slot.
fn check<'de, T: Deserialize<'de>>(raw: &'de RawValue) -> Result<(), ()> {
    let mut deserializer = serde_json::Deserializer::from_str(raw.get());
    let read: Result<Checked<T>, _> = serde_path_to_error::deserialize(strict::Objects(&mut deserializer));
    let Err(error) = read else {
        return Ok(());
    };
    let path = error.path().clone();
    let error = error.into_inner();
    let here = span_of(raw.get());
    FAULT.with(|slot| {
        let mut slot = slot.borrow_mut();
        // A row inside this one stopped it: add this row's place in
        // front of that row's.
        let deeper = error.to_string().starts_with(ROW_FAULT)
            && slot.as_ref().is_some_and(|fault| inside(fault.base, here));
        match slot.as_mut() {
            Some(fault) if deeper => fault.steps.insert(0, path),
            _ => {
                *slot = Some(Fault {
                    steps: vec![path],
                    error,
                    base: here,
                })
            }
        }
    });
    Err(())
}

impl<'de: 'a, 'a, T: Deserialize<'de>> Deserialize<'de> for Rows<'a, T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = <&'de RawValue>::deserialize(deserializer)?;
        check::<T>(raw).map_err(|()| de::Error::custom(ROW_FAULT))?;
        Ok(Rows {
            source: Source::Text(raw),
            items: PhantomData,
        })
    }
}

impl<'a, T: Serialize + Deserialize<'a>> Serialize for Rows<'a, T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if let Source::Text(raw) = &self.source {
            return raw.serialize(serializer);
        }
        let mut seq = serializer.serialize_seq(None)?;
        let mut failed: Option<S::Error> = None;
        let walked = self.each(|_, item: T| match seq.serialize_element(&item) {
            Ok(()) => Ok(()),
            Err(error) => {
                failed = Some(error);
                Err(())
            }
        });
        if let Some(error) = failed {
            return Err(error);
        }
        match walked {
            Ok(()) => seq.end(),
            Err(RowsError::Save(error)) => Err(ser::Error::custom(error)),
            Err(RowsError::Read(error)) => Err(ser::Error::custom(error)),
            Err(RowsError::Item(())) => Err(ser::Error::custom("a row stopped")),
        }
    }
}
