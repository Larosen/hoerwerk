//! Python's difflib.SequenceMatcher(None, a, b).ratio() – same algorithm, including the "autojunk" heuristic,
//! so match scores are identical to the Python version.

use std::collections::HashMap;

pub fn ratio(a: &str, b: &str) -> f64 {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let total = a.len() + b.len();
    if total == 0 {
        return 1.0;
    }
    let matches: usize = matching_blocks(&a, &b).iter().map(|m| m.2).sum();
    2.0 * matches as f64 / total as f64
}

fn b2j(b: &[char]) -> HashMap<char, Vec<usize>> {
    let mut map: HashMap<char, Vec<usize>> = HashMap::new();
    for (i, c) in b.iter().enumerate() {
        map.entry(*c).or_default().push(i);
    }
    // autojunk: in sequences of 200+ items, items making up more than 1% are "popular" and ignored
    let n = b.len();
    if n >= 200 {
        let ntest = n / 100 + 1;
        map.retain(|_, idxs| idxs.len() <= ntest);
    }
    map
}

fn longest(
    a: &[char],
    b: &[char],
    map: &HashMap<char, Vec<usize>>,
    alo: usize,
    ahi: usize,
    blo: usize,
    bhi: usize,
) -> (usize, usize, usize) {
    let (mut besti, mut bestj, mut bestsize) = (alo, blo, 0usize);
    let mut j2len: HashMap<usize, usize> = HashMap::new();
    #[allow(clippy::needless_range_loop)] // the index is part of the result
    for i in alo..ahi {
        let mut newj2len: HashMap<usize, usize> = HashMap::new();
        if let Some(idxs) = map.get(&a[i]) {
            for &j in idxs {
                if j < blo {
                    continue;
                }
                if j >= bhi {
                    break;
                }
                let k = j.checked_sub(1).and_then(|p| j2len.get(&p)).copied().unwrap_or(0) + 1;
                newj2len.insert(j, k);
                if k > bestsize {
                    besti = i + 1 - k;
                    bestj = j + 1 - k;
                    bestsize = k;
                }
            }
        }
        j2len = newj2len;
    }
    // isjunk is None: the best match grows over any equal neighbours (popular ones included)
    while besti > alo && bestj > blo && a[besti - 1] == b[bestj - 1] {
        besti -= 1;
        bestj -= 1;
        bestsize += 1;
    }
    while besti + bestsize < ahi && bestj + bestsize < bhi && a[besti + bestsize] == b[bestj + bestsize] {
        bestsize += 1;
    }
    (besti, bestj, bestsize)
}

fn matching_blocks(a: &[char], b: &[char]) -> Vec<(usize, usize, usize)> {
    let map = b2j(b);
    let mut queue = vec![(0, a.len(), 0, b.len())];
    let mut blocks = Vec::new();
    while let Some((alo, ahi, blo, bhi)) = queue.pop() {
        let (i, j, k) = longest(a, b, &map, alo, ahi, blo, bhi);
        if k > 0 {
            blocks.push((i, j, k));
            if alo < i && blo < j {
                queue.push((alo, i, blo, j));
            }
            if i + k < ahi && j + k < bhi {
                queue.push((i + k, ahi, j + k, bhi));
            }
        }
    }
    blocks.sort();
    blocks
}
