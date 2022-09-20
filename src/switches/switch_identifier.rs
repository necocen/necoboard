use rustkbd::keyboard;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SwitchIdentifier {
    pub row: u8,
    pub col: u8,
}

impl From<[u8; 2]> for SwitchIdentifier {
    fn from(value: [u8; 2]) -> Self {
        SwitchIdentifier {
            row: value[0],
            col: value[1],
        }
    }
}

impl From<SwitchIdentifier> for [u8; 2] {
    fn from(value: SwitchIdentifier) -> Self {
        [value.row, value.col]
    }
}

impl keyboard::KeySwitchIdentifier<2> for SwitchIdentifier {}
