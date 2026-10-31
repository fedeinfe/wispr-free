//! How a list on show changes.

/// One change to a list. A front end keeps the rows it isn't told about, with their focus and
/// their place on screen, so a change says no more than it has to.
#[derive(Debug, Clone, PartialEq)]
pub enum Rows<T> {
    All(Vec<T>),
    One(usize, T),
    Add(T),
    Insert(usize, T),
}

impl<T> Rows<T> {
    /// A row past the end is ignored, as a list on screen ignores it.
    pub fn apply(self, rows: &mut Vec<T>) {
        match self {
            Rows::All(all) => *rows = all,
            Rows::One(index, row) => {
                if let Some(known) = rows.get_mut(index) {
                    *known = row;
                }
            }
            Rows::Add(row) => rows.push(row),
            Rows::Insert(index, row) => rows.insert(index, row),
        }
    }
}
