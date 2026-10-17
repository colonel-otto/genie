//! The MSVC standard library's algorithms as build 158041 compiled them, where their order shows
//! in the simulation: `std::sort` is an introsort whose order of equal elements depends on the
//! algorithm, so a port must run the same steps.
//!
//! `pred(a, b)` is the sort's comparator: true when `a` goes before `b`.

/// `std::sort` (`_Sort_unchecked`, the tile pathfinder's at `0x140f9c510`): quicksort by the
/// median guess while the range holds more than 32 elements and divisions are left, a heap sort
/// when they run out, an insertion sort for the rest.
pub fn sort<T: Copy>(v: &mut [T], pred: &impl Fn(T, T) -> bool) {
    let ideal = v.len() as isize;
    sort_unchecked(v, 0, v.len(), ideal, pred);
}

/// `_ISORT_MAX`.
const INSERTION_MAX: usize = 32;

fn sort_unchecked<T: Copy>(
    v: &mut [T],
    mut first: usize,
    mut last: usize,
    mut ideal: isize,
    pred: &impl Fn(T, T) -> bool,
) {
    loop {
        if last - first <= INSERTION_MAX {
            insertion_sort(v, first, last, pred);
            return;
        }
        if ideal <= 0 {
            make_heap(&mut v[first..last], pred);
            sort_heap(&mut v[first..last], pred);
            return;
        }
        let (mid_first, mid_last) = partition_by_median_guess(v, first, last, pred);
        ideal = (ideal >> 1) + (ideal >> 2);
        if mid_first - first < last - mid_last {
            sort_unchecked(v, first, mid_first, ideal, pred);
            first = mid_last;
        } else {
            sort_unchecked(v, mid_last, last, ideal, pred);
            last = mid_first;
        }
    }
}

/// `_Insertion_sort_unchecked`: equal elements keep their order.
fn insertion_sort<T: Copy>(v: &mut [T], first: usize, last: usize, pred: &impl Fn(T, T) -> bool) {
    if first == last {
        return;
    }
    for mid in first + 1..last {
        let val = v[mid];
        if pred(val, v[first]) {
            v.copy_within(first..mid, first + 1);
            v[first] = val;
        } else {
            let mut hole = mid;
            while pred(val, v[hole - 1]) {
                v[hole] = v[hole - 1];
                hole -= 1;
            }
            v[hole] = val;
        }
    }
}

/// `_Med3_unchecked`: the median of three to the middle.
fn med3<T: Copy>(v: &mut [T], first: usize, mid: usize, last: usize, pred: &impl Fn(T, T) -> bool) {
    if pred(v[mid], v[first]) {
        v.swap(mid, first);
    }
    if pred(v[last], v[mid]) {
        v.swap(last, mid);
        if pred(v[mid], v[first]) {
            v.swap(mid, first);
        }
    }
}

/// `_Guess_median_unchecked` (`0x140f9c040`): Tukey's ninther above 40 elements; `last` is the
/// range's last element.
fn guess_median<T: Copy>(
    v: &mut [T],
    first: usize,
    mid: usize,
    last: usize,
    pred: &impl Fn(T, T) -> bool,
) {
    let count = last - first;
    if 40 < count {
        let step = (count + 1) >> 3;
        let two_step = step << 1;
        med3(v, first, first + step, first + two_step, pred);
        med3(v, mid - step, mid, mid + step, pred);
        med3(v, last - two_step, last - step, last, pred);
        med3(v, first + step, mid, last - step, pred);
    } else {
        med3(v, first, mid, last, pred);
    }
}

/// `_Partition_by_median_guess_unchecked` (`0x140f9c210`): the range of elements equal to the
/// pivot, those before it in front, those after it behind.
fn partition_by_median_guess<T: Copy>(
    v: &mut [T],
    first: usize,
    last: usize,
    pred: &impl Fn(T, T) -> bool,
) -> (usize, usize) {
    let mid = first + ((last - first) >> 1);
    guess_median(v, first, mid, last - 1, pred);
    let mut pfirst = mid;
    let mut plast = pfirst + 1;
    while first < pfirst && !pred(v[pfirst - 1], v[pfirst]) && !pred(v[pfirst], v[pfirst - 1]) {
        pfirst -= 1;
    }
    while plast < last && !pred(v[plast], v[pfirst]) && !pred(v[pfirst], v[plast]) {
        plast += 1;
    }
    let mut gfirst = plast;
    let mut glast = pfirst;
    loop {
        while gfirst < last {
            if pred(v[pfirst], v[gfirst]) {
            } else if pred(v[gfirst], v[pfirst]) {
                break;
            } else {
                if plast != gfirst {
                    v.swap(plast, gfirst);
                }
                plast += 1;
            }
            gfirst += 1;
        }
        while first < glast {
            if pred(v[glast - 1], v[pfirst]) {
            } else if pred(v[pfirst], v[glast - 1]) {
                break;
            } else {
                pfirst -= 1;
                if pfirst != glast - 1 {
                    v.swap(pfirst, glast - 1);
                }
            }
            glast -= 1;
        }
        if glast == first && gfirst == last {
            return (pfirst, plast);
        }
        if glast == first {
            if plast != gfirst {
                v.swap(pfirst, plast);
            }
            plast += 1;
            v.swap(pfirst, gfirst);
            pfirst += 1;
            gfirst += 1;
        } else if gfirst == last {
            glast -= 1;
            pfirst -= 1;
            if glast != pfirst {
                v.swap(glast, pfirst);
            }
            plast -= 1;
            v.swap(pfirst, plast);
        } else {
            glast -= 1;
            v.swap(gfirst, glast);
            gfirst += 1;
        }
    }
}

/// `_Push_heap_by_index`.
fn push_heap_by_index<T: Copy>(
    v: &mut [T],
    mut hole: usize,
    top: usize,
    val: T,
    pred: &impl Fn(T, T) -> bool,
) {
    while top < hole {
        let idx = (hole - 1) >> 1;
        if !pred(v[idx], val) {
            break;
        }
        v[hole] = v[idx];
        hole = idx;
    }
    v[hole] = val;
}

/// `_Pop_heap_hole_by_index`: the hole moves down to the larger child, then `val` goes up.
fn pop_heap_hole_by_index<T: Copy>(
    v: &mut [T],
    mut hole: usize,
    bottom: usize,
    val: T,
    pred: &impl Fn(T, T) -> bool,
) {
    let top = hole;
    let mut idx = hole;
    let max_non_leaf = (bottom.wrapping_sub(1)) >> 1;
    while idx < max_non_leaf {
        idx = 2 * idx + 2;
        if pred(v[idx], v[idx - 1]) {
            idx -= 1;
        }
        v[hole] = v[idx];
        hole = idx;
    }
    if idx == max_non_leaf && bottom.is_multiple_of(2) {
        v[hole] = v[bottom - 1];
        hole = bottom - 1;
    }
    push_heap_by_index(v, hole, top, val, pred);
}

/// `_Make_heap_unchecked`.
fn make_heap<T: Copy>(v: &mut [T], pred: &impl Fn(T, T) -> bool) {
    let bottom = v.len();
    let mut hole = bottom >> 1;
    while 0 < hole {
        hole -= 1;
        let val = v[hole];
        pop_heap_hole_by_index(v, hole, bottom, val, pred);
    }
}

/// `_Sort_heap_unchecked`.
fn sort_heap<T: Copy>(v: &mut [T], pred: &impl Fn(T, T) -> bool) {
    let mut last = v.len();
    while 2 <= last {
        last -= 1;
        let val = v[last];
        v[last] = v[0];
        pop_heap_hole_by_index(v, 0, last, val, pred);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pairs sorted by their key, descending, as the pathfinder sorts its open nodes.
    fn by_key(v: &mut [(u32, usize)]) {
        sort(v, &|a: (u32, usize), b: (u32, usize)| a.0 > b.0);
    }

    #[test]
    fn a_short_range_sorts_by_insertion_and_keeps_equal_elements_in_order() {
        let mut v: Vec<(u32, usize)> = [3, 1, 3, 2, 1, 3].iter().copied().zip(0..).collect();
        by_key(&mut v);
        assert_eq!(v, [(3, 0), (3, 2), (3, 5), (2, 3), (1, 1), (1, 4)]);
    }

    #[test]
    fn a_long_range_is_sorted_and_keeps_every_element() {
        // a fixed pseudo-random sequence with many equal keys (an LCG)
        let mut x = 12345u32;
        let mut v: Vec<(u32, usize)> = (0..500)
            .map(|i| {
                x = x.wrapping_mul(1103515245).wrapping_add(12345);
                ((x >> 16) % 37, i)
            })
            .collect();
        let mut expected = v.clone();
        by_key(&mut v);
        assert!(v.windows(2).all(|w| w[0].0 >= w[1].0));
        let mut got = v.clone();
        got.sort();
        expected.sort();
        assert_eq!(got, expected);
    }

    #[test]
    fn the_heap_sort_sorts_when_no_divisions_are_left() {
        let mut v: Vec<(u32, usize)> = (0..100).map(|i| ((i * 7919 % 100) as u32, i)).collect();
        let n = v.len();
        sort_unchecked(&mut v, 0, n, 0, &|a: (u32, usize), b: (u32, usize)| {
            a.0 > b.0
        });
        assert!(v.windows(2).all(|w| w[0].0 >= w[1].0));
    }
}
