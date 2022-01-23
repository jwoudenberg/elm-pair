// Tust nightlies already contain a `intersperse` iterator. Once that lands
// in stable we should switch over.
pub trait Intersperse: Iterator {
    fn my_intersperse(self, separator: Self::Item) -> IntersperseState<Self>
    where
        Self::Item: Clone,
        Self: Sized;
}

impl<I: Iterator> Intersperse for I {
    fn my_intersperse(self, separator: Self::Item) -> IntersperseState<I> {
        IntersperseState {
            iterator: self.peekable(),
            separator,
            separator_is_next: false,
        }
    }
}

pub struct IntersperseState<I: Iterator> {
    iterator: std::iter::Peekable<I>,
    separator: I::Item,
    separator_is_next: bool,
}

impl<I: Iterator> Iterator for IntersperseState<I>
where
    I::Item: Clone,
{
    type Item = I::Item;

    fn next(&mut self) -> Option<Self::Item> {
        if self.iterator.peek().is_none() {
            None
        } else if self.separator_is_next {
            self.separator_is_next = false;
            Some(self.separator.clone())
        } else {
            self.separator_is_next = true;
            self.iterator.next()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_iterator() {
        let empty: Vec<bool> = vec![];
        assert_eq!(
            empty
                .clone()
                .into_iter()
                .my_intersperse(false)
                .collect::<Vec<bool>>(),
            empty,
        );
    }

    #[test]
    fn single_element_iterator() {
        let singleton: Vec<bool> = vec![true];
        assert_eq!(
            singleton
                .clone()
                .into_iter()
                .my_intersperse(false)
                .collect::<Vec<bool>>(),
            singleton,
        );
    }

    #[test]
    fn many_element_iterator() {
        let vec: Vec<bool> = vec![true, true, true];
        assert_eq!(
            vec.into_iter().my_intersperse(false).collect::<Vec<bool>>(),
            vec![true, false, true, false, true],
        );
    }
}
