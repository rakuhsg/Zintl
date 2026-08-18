use crate::store::StoreId;
use std::any::{Any, TypeId};
use std::collections::HashMap;

pub struct Sequence<T> {
    items: Vec<T>,
}

impl<T> Sequence<T> {
    fn new() -> Self {
        Self { items: Vec::new() }
    }

    pub fn get(&self, index: usize) -> Option<&T> {
        self.items.get(index)
    }

    pub fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        self.items.get_mut(index)
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    fn push(&mut self, item: T) -> usize {
        let index = self.items.len();
        self.items.push(item);
        index
    }
}

pub struct Arena {
    sequences: Vec<Box<dyn Any>>,
    sequence_ids: HashMap<TypeId, usize>,
}

impl Arena {
    pub fn new() -> Self {
        Self {
            sequences: Vec::new(),
            sequence_ids: HashMap::new(),
        }
    }

    pub fn sequence<T: 'static>(&self) -> Option<&Sequence<T>> {
        let sequence_id = *self.sequence_ids.get(&TypeId::of::<T>())?;
        self.sequences
            .get(sequence_id)?
            .downcast_ref::<Sequence<T>>()
    }

    pub fn sequence_mut<T: 'static>(&mut self) -> Option<&mut Sequence<T>> {
        let sequence_id = *self.sequence_ids.get(&TypeId::of::<T>())?;
        self.sequences
            .get_mut(sequence_id)?
            .downcast_mut::<Sequence<T>>()
    }

    pub fn get<T: 'static>(&self, id: StoreId) -> Option<&T> {
        self.sequences
            .get(id.sequence_id)?
            .downcast_ref::<Sequence<T>>()?
            .get(id.index)
    }

    pub fn get_mut<T: 'static>(&mut self, id: StoreId) -> Option<&mut T> {
        self.sequences
            .get_mut(id.sequence_id)?
            .downcast_mut::<Sequence<T>>()?
            .get_mut(id.index)
    }

    pub fn insert<T: 'static>(&mut self, item: T) -> StoreId {
        let type_id = TypeId::of::<T>();

        if let Some(sequence_id) = self.sequence_ids.get(&type_id).copied() {
            let sequence = self.sequences[sequence_id]
                .downcast_mut::<Sequence<T>>()
                .expect("sequence type must match its TypeId");
            let index = sequence.push(item);
            return StoreId { sequence_id, index };
        }

        let sequence_id = self.sequences.len();
        let mut sequence = Sequence::new();
        let index = sequence.push(item);
        self.sequences.push(Box::new(sequence));
        self.sequence_ids.insert(type_id, sequence_id);

        StoreId { sequence_id, index }
    }
}

impl Default for Arena {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_values_into_sequences_by_type() {
        // Values of the same type share a sequence while other types use another sequence.
        let mut arena = Arena::new();
        let first_number = arena.insert(35_i32);
        let second_number = arena.insert(12_i32);
        let text = arena.insert(String::from("string"));

        assert_eq!(first_number.sequence_id, second_number.sequence_id);
        assert_ne!(first_number.sequence_id, text.sequence_id);
        let first_value = arena.get(first_number).unwrap();
        let second_value = arena.get(second_number).unwrap();
        assert_eq!((*first_value, *second_value), (35, 12));
        assert_eq!(arena.get(text).map(String::as_str), Some("string"));
    }

    #[test]
    fn retrieves_a_sequence_by_its_value_type() {
        // TypeId lookup returns the complete typed sequence and rejects absent types.
        let mut arena = Arena::new();
        arena.insert(String::from("first"));
        arena.insert(String::from("second"));

        let strings = arena.sequence_mut::<String>().unwrap();
        *strings.get_mut(1).unwrap() = String::from("updated");

        let strings = arena.sequence::<String>().unwrap();
        assert_eq!(strings.len(), 2);
        assert_eq!(strings.get(0).map(String::as_str), Some("first"));
        assert_eq!(strings.get(1).map(String::as_str), Some("updated"));
        assert!(arena.sequence::<u64>().is_none());
    }

    #[test]
    fn mutates_values_and_rejects_invalid_access() {
        // Mutable access updates a value without accepting a wrong type or invalid index.
        let mut arena = Arena::new();
        let number = arena.insert(35_i32);

        *arena.get_mut(number).unwrap() = 42;

        assert_eq!(arena.get(number), Some(&42));
        assert!(arena.get::<String>(number).is_none());
        assert!(
            arena
                .get::<i32>(StoreId {
                    sequence_id: number.sequence_id,
                    index: usize::MAX,
                })
                .is_none()
        );
    }
}
