use std::collections::VecDeque;
use std::future::Future;

/// Performs an efficient search of elements to locate the nearest element to `target` without going
/// over. Assumes there are `element_count` elements in a rotated sorted array with zero or many
/// `None` values at the pivot point. Returns `None` if there are no values less than the `target`.
pub(crate) async fn search<F, V>(
    element_count: usize,
    target: V,
    mut f: impl FnMut(usize) -> F,
) -> crate::result::Result<Option<usize>>
where
    F: Future<Output = crate::result::Result<Option<V>>>,
    V: PartialOrd + Clone,
{
    if element_count == 0 {
        return Ok(None);
    }

    let some_target = Some(&target);
    let mut nearest = None;

    let mut first_value = f(0).await?;
    let mut first_value_ref = first_value.as_ref();

    if first_value_ref == some_target {
        return Ok(Some(0));
    }

    let mut low = 0;
    let mut high = element_count;

    // First, locate any value in the array to use as a reference point via repeated bisection
    let mut queue = VecDeque::from([(0, element_count - 1)]);
    while !queue.is_empty() {
        if let Some((start, end)) = queue.pop_front() {
            if start > end {
                continue;
            }

            let mid = (start + end) / 2;
            let mid_value = f(mid).await?;
            let mid_value_ref = mid_value.as_ref();

            // If this value is None, continue the bisection
            if mid_value_ref.is_none() {
                queue.push_back((mid + 1, end));
                if mid > 0 {
                    queue.push_back((start, mid - 1));
                }
                continue;
            }

            if mid_value_ref <= some_target {
                nearest = Some(mid);
            }

            if mid_value_ref == some_target {
                return Ok(nearest);
            }

            if should_search_right(first_value_ref, mid_value_ref, some_target) {
                low = mid + 1;
            } else {
                high = mid;
            }
        }

        break;
    }

    if low >= high {
        return Ok(nearest);
    }

    // Move the low pointer to the first non-None value
    first_value = f(low).await?;
    first_value_ref = first_value.as_ref();

    // Now that we have a reference point, we can perform a binary search for the target
    while low < high {
        let mid = low + (high - low) / 2;

        let value = f(mid).await?;
        let value_ref = value.as_ref();

        if value_ref.is_some() && value_ref <= some_target {
            nearest = Some(mid);
        }

        if value_ref == some_target {
            return Ok(Some(mid));
        }

        if should_search_right(first_value_ref, value_ref, some_target) {
            low = mid + 1;
        } else {
            high = mid;
        }
    }

    Ok(nearest)
}

/// Returns `true` if the search should continue right, `false` if it should continue left.
fn should_search_right<V>(first: V, value: V, target: V) -> bool
where
    V: PartialOrd,
{
    let first_wrapped = first > value;
    let target_wrapped = target < first;

    if value < target {
        !first_wrapped || target_wrapped
    } else {
        first_wrapped && !target_wrapped
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    mod binary_search {
        use super::*;

        macro_rules! test {
            ($name:ident, $elements:expr, $target:expr, $expected:expr) => {
                #[tokio::test]
                async fn $name() {
                    let result = search(
                        $elements.len(),
                        $target,
                        |i| async move { Ok($elements[i]) },
                    )
                    .await
                    .unwrap();
                    assert_eq!(result, $expected);
                }
            };
        }

        test!(empty, vec![] as Vec<Option<usize>>, 0, None);
        test!(single, vec![Some(0)], 0, Some(0));
        test!(single_under, vec![Some(1)], 0, None);
        test!(single_over, vec![Some(0)], 1, Some(0));

        test!(double_match, vec![Some(0), Some(1)], 0, Some(0));

        test!(double_over, vec![Some(0), Some(1)], 2, Some(1));

        test!(double_under, vec![Some(1), Some(2)], 0, None);

        test!(double_middle, vec![Some(0), Some(2)], 1, Some(0));

        test!(double_middle_over, vec![Some(0), Some(2)], 3, Some(1));

        test!(
            filled,
            vec![
                Some(0),
                Some(1),
                Some(2),
                Some(3),
                Some(4),
                Some(5),
                Some(6),
                Some(7),
                Some(8),
                Some(9),
            ],
            5,
            Some(5)
        );

        test!(
            filled_nonmatch,
            vec![
                Some(0),
                Some(1),
                Some(2),
                Some(3),
                Some(6),
                Some(7),
                Some(8),
                Some(9),
            ],
            5,
            Some(3)
        );

        test!(
            none_end,
            vec![
                Some(0),
                Some(1),
                Some(2),
                Some(3),
                Some(4),
                Some(5),
                Some(6),
                None,
                None,
                None,
            ],
            8,
            Some(6)
        );

        test!(
            none_beginning_no_match,
            vec![
                None,
                None,
                None,
                Some(2),
                Some(3),
                Some(4),
                Some(5),
                Some(6),
                Some(7),
                Some(8),
            ],
            1,
            None
        );

        test!(
            none_beginning_match,
            vec![
                None,
                None,
                None,
                Some(2),
                Some(3),
                Some(4),
                Some(5),
                Some(6),
                Some(7),
                Some(8),
            ],
            3,
            Some(4)
        );

        test!(
            none_wrapping_match,
            vec![
                None,
                Some(2),
                Some(3),
                Some(4),
                Some(5),
                Some(6),
                Some(7),
                Some(8),
                None,
                None,
            ],
            3,
            Some(2)
        );

        test!(
            none_wrapping_no_match,
            vec![
                None,
                Some(2),
                Some(3),
                Some(4),
                Some(5),
                Some(6),
                Some(7),
                Some(8),
                None,
                None,
            ],
            1,
            None
        );

        test!(
            none_wrapping_non_exact_match,
            vec![
                None,
                Some(2),
                Some(3),
                Some(4),
                Some(5),
                Some(6),
                Some(7),
                Some(8),
                None,
                None,
            ],
            10,
            Some(7)
        );

        test!(
            wrapping_match_start,
            vec![
                Some(6),
                Some(7),
                Some(8),
                Some(2),
                Some(3),
                Some(4),
                Some(5),
            ],
            7,
            Some(1)
        );

        test!(
            wrapping_match_end,
            vec![
                Some(6),
                Some(7),
                Some(8),
                Some(2),
                Some(3),
                Some(4),
                Some(5),
            ],
            4,
            Some(5)
        );

        test!(
            wrapping_no_match,
            vec![
                Some(6),
                Some(7),
                Some(8),
                Some(2),
                Some(3),
                Some(4),
                Some(5),
            ],
            1,
            None
        );

        test!(
            wrapping_none_match_start,
            vec![
                Some(6),
                Some(7),
                Some(8),
                None,
                None,
                None,
                Some(2),
                Some(3),
                Some(4),
                Some(5),
            ],
            7,
            Some(1)
        );

        test!(
            wrapping_none_no_match,
            vec![
                Some(6),
                Some(7),
                Some(8),
                None,
                None,
                None,
                Some(2),
                Some(3),
                Some(4),
                Some(5),
            ],
            1,
            None
        );

        test!(
            wrapping_none_match_end,
            vec![
                Some(6),
                Some(7),
                Some(8),
                None,
                None,
                None,
                Some(2),
                Some(3),
                Some(4),
                Some(5),
            ],
            2,
            Some(6)
        );

        test!(
            all_none,
            vec![None, None, None, None] as Vec<Option<usize>>,
            5,
            None
        );

        test!(
            none_middle,
            vec![None, Some(1), Some(3), None, None, None, None, None],
            100,
            Some(2)
        );
    }

    mod should_search_right {
        use super::*;

        macro_rules! test {
            ($name:ident, $first:expr, $value:expr, $target:expr, $expected:expr) => {
                #[test]
                fn $name() {
                    assert_eq!(should_search_right($first, $value, $target), $expected);
                }
            };
        }

        test!(simple, 2, 5, 8, true);
        test!(repeated, 2, 2, 5, true);
        test!(preceding, 1, 8, 2, false);
        test!(wrapped_below_pivot, 8, 2, 5, true);
        test!(wrapped_above_pivot, 8, 5, 9, false);
        test!(wrapped_preceding, 6, 5, 2, false);
    }
}
