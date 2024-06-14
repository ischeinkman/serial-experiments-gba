use core::{
    cell::{Cell, UnsafeCell},
    mem::MaybeUninit,
    ops::{Add, AddAssign},
};

use agb::{
    external::critical_section::{self, CriticalSection, Mutex},
    fixnum::{num, Num, Rect, Vector2D},
};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

impl Direction {
    pub const fn is_vertical(self) -> bool {
        matches!(self, Direction::Up | Direction::Down)
    }
    pub const fn is_horizontal(self) -> bool {
        !self.is_vertical()
    }
    pub const fn flipped(self) -> Direction {
        use Direction::*;
        match self {
            Up => Down,
            Down => Up,
            Left => Right,
            Right => Left,
        }
    }
    pub fn scaled_vec(self, scaler: N) -> VectType {
        use Direction::*;
        let x = match self {
            Right => scaler,
            Left => -scaler,
            Up | Down => N::from_raw(0),
        };
        let y = match self {
            Down => scaler,
            Up => -scaler,
            Left | Right => N::from_raw(0),
        };
        VectType::new(x, y)
    }
}

pub const N_FRAC_BITS: usize = 12;
pub const MAX_FRAC_PORTION: i32 = (1 << N_FRAC_BITS) - 1;
pub type N = Num<i32, N_FRAC_BITS>;
pub type VectType = Vector2D<N>;
pub type RectType = Rect<N>;

pub trait RectExt {
    fn tl(&self) -> VectType;
    fn tr(&self) -> VectType;
    fn bl(&self) -> VectType;
    fn br(&self) -> VectType;
    fn center(&self) -> VectType;
}

impl RectExt for RectType {
    fn tl(&self) -> VectType {
        self.position
    }
    fn tr(&self) -> VectType {
        VectType::new(self.position.x + self.size.x, self.position.y)
    }
    fn bl(&self) -> VectType {
        VectType::new(self.position.x, self.position.y + self.size.y)
    }
    fn br(&self) -> VectType {
        self.position + self.size
    }
    fn center(&self) -> VectType {
        self.position + self.size / 2
    }
}

pub fn split_mut_at<T>(base: &mut [T], n: usize) -> Option<(&[T], &mut T, &[T])> {
    let (left, elm_right) = base.split_at_mut(n);
    let (elm, right) = elm_right.split_first_mut()?;
    Some((left, elm, right))
}

pub trait Hitbox {
    fn pos(&self) -> VectType;
    fn size(&self) -> VectType;
    fn hitbox(&self) -> RectType {
        RectType::new(self.pos(), self.size())
    }
    fn collides(&self, other: &impl Hitbox) -> bool {
        self.hitbox().touches(other.hitbox())
    }
    fn next_hitbox(&self, next_pos: VectType) -> RectType {
        RectType::new(next_pos, self.size())
    }
}

impl Hitbox for RectType {
    fn pos(&self) -> VectType {
        self.position
    }
    fn size(&self) -> VectType {
        self.size
    }
    fn hitbox(&self) -> RectType {
        *self
    }
}

pub const fn n_from_bit(bit: usize) -> N {
    n_from_bits(&[bit])
}

pub const fn n_from_bits(bits: &[usize]) -> N {
    let mut idx = 0;
    let mut retvl_raw = 0;
    while idx < bits.len() {
        assert!(N_FRAC_BITS >= bits[idx]);
        retvl_raw |= 1 << (N_FRAC_BITS - bits[idx]);
        idx += 1;
    }
    N::from_raw(retvl_raw)
}

pub const fn n_from_parts(whole: i32, frac: i32) -> N {
    assert!(frac < (1 << N_FRAC_BITS));
    assert!(whole.abs() < (1 << (31 - N_FRAC_BITS)));
    N::from_raw(whole << N_FRAC_BITS | frac)
}

pub fn step_to(cur: N, step: N, goal: N) -> N {
    let step = if cur > goal { -step.abs() } else { step.abs() };
    let next = cur + step;
    if cur < goal && next < goal {
        next
    } else if cur < goal {
        goal
    } else if cur > goal && next > goal {
        next
    } else {
        goal
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct AlignedVec {
    mag: N,
    dir: Direction,
}

impl AlignedVec {
    pub const fn new_unchecked(mag: N, dir: Direction) -> Self {
        Self { mag, dir }
    }
    pub fn new(n: N, dir: Direction) -> Self {
        let (n, dir) = if n < num!(0.0) {
            (n.abs(), dir.flipped())
        } else {
            (n, dir)
        };
        Self::new_unchecked(n, dir)
    }
    pub const fn zero(dir: Direction) -> Self {
        Self::new_unchecked(N::from_raw(0), dir)
    }
    pub fn magnitude(&self) -> N {
        self.mag
    }

    pub fn x(&self) -> N {
        use Direction::*;
        match self.dir {
            Up | Down => N::from_raw(0),
            Right => self.mag,
            Left => -self.mag,
        }
    }
    pub fn y(&self) -> N {
        use Direction::*;
        match self.dir {
            Left | Right => N::from_raw(0),
            Up => -self.mag,
            Down => self.mag,
        }
    }
    pub fn step_to(&self, step: N, goal: N) -> Self {
        Self {
            mag: step_to(self.mag, step, goal),
            dir: self.dir,
        }
    }
    pub fn step_to_dir(&self, step: N, goal: AlignedVec) -> Self {
        if goal.mag.to_raw() == 0 {
            return self.step_to(step, goal.mag);
        }
        let mag = if goal.dir == self.dir {
            self.mag
        } else if goal.dir == self.dir.flipped() {
            -self.mag
        } else {
            N::from_raw(0)
        };
        Self::new(step_to(mag, step, goal.mag), goal.dir)
    }
}

impl AddAssign<AlignedVec> for VectType {
    fn add_assign(&mut self, rhs: AlignedVec) {
        self.x += rhs.x();
        self.y += rhs.y();
    }
}

impl Add<AlignedVec> for VectType {
    type Output = VectType;
    fn add(self, rhs: AlignedVec) -> Self::Output {
        VectType::new(self.x + rhs.x(), self.y + rhs.y())
    }
}

#[inline(always)]
pub const fn read_bit(value: u16, n: u8) -> bool {
    value & (1 << n) != 0
}
#[inline(always)]
pub const fn write_bit(v: u16, n: u8, bit: bool) -> u16 {
    (v & !(1 << n)) | ((bit as u16) << n)
}
#[inline(always)]
pub const fn read_bit_u8(value: u8, n: u8) -> bool {
    value & (1 << n) != 0
}
#[inline(always)]
pub const fn write_bit_u8(v: u8, n: u8, bit: bool) -> u8 {
    (v & !(1 << n)) | ((bit as u8) << n)
}

pub struct GbaCell<T> {
    inner: Mutex<Cell<T>>,
}

impl<T> GbaCell<T> {
    pub const fn new(value: T) -> Self {
        Self {
            inner: Mutex::new(Cell::new(value)),
        }
    }
    pub fn swap(&self, value: T) -> T {
        critical_section::with(|cs| self.inner.borrow(cs).replace(value))
    }
    pub fn get_mut(&mut self) -> &mut T {
        self.inner.get_mut().get_mut()
    }
    pub fn swap_if<F>(&self, value: T, condition: F) -> Result<T, T>
    where
        F: FnOnce(&T) -> bool,
    {
        critical_section::with(|cs| {
            let old = self.inner.borrow(cs).replace(value);
            if condition(&old) {
                Ok(old)
            } else {
                let value = self.inner.borrow(cs).replace(old);
                Err(value)
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agb::Gba;

    #[test_case]
    fn test_n_stepper(_gba: &mut Gba) {
        let tests = [
            // Test basic
            (0.0, 0.2, 1.0, 0.2),
            // Step from previous
            (0.7, 0.2, 1.0, 0.9),
            // Overstep
            (0.9, 0.2, 1.0, 1.0),
            // Clamping
            (1.1, 0.2, 1.0, 1.0),
            // Direction change
            (0.9, 0.2, -1.0, 0.7),
            // Direction change 2
            (0.9, -0.2, 1.0, 1.0),
        ];
        for (idx, (cur, step, goal, expected)) in tests.into_iter().enumerate() {
            let cur = N::from_f32(cur);
            let step = N::from_f32(step);
            let goal = N::from_f32(goal);
            let expected = N::from_f32(expected);
            let actual = step_to(cur, step, goal);
            assert_eq!(expected, actual, "Error in test: {}", idx);
        }
    }
}
