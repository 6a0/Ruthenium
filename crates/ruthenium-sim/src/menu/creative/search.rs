//! The creative inventory's search (`SessionSearchTrees`' creative trees): vanilla's
//! `SuffixArray` (with fastutil's `Arrays.quickSort`, since the array it sorts is only
//! sorted as far as its longest string allows and a search reads it as it lies),
//! `IdentifierSearchTree`, `IdSearchTree` and `FullTextSearchTree`. Elements are
//! numbered by their place in the list the tree was made from; a search gives them in
//! that order. Text is in UTF-16 units, as Java's strings are.

/// `SuffixArray`.
#[derive(Debug, Clone, Default)]
pub struct SuffixArray {
    /// The element of each string added (`list`).
    list: Vec<usize>,
    chars: Vec<i32>,
    word_starts: Vec<usize>,
    suffix_to_t: Vec<usize>,
    offsets: Vec<usize>,
    max_string_length: usize,
}

/// The sort's state: `lefts`, `rights` and `reverse` move together.
struct Sorting {
    lefts: Vec<i32>,
    rights: Vec<i32>,
    reverse: Vec<usize>,
}

impl Sorting {
    fn compare(&self, a: usize, b: usize) -> std::cmp::Ordering {
        if self.lefts[a] == self.lefts[b] {
            self.rights[a].cmp(&self.rights[b])
        } else {
            self.lefts[a].cmp(&self.lefts[b])
        }
    }

    fn swap(&mut self, a: usize, b: usize) {
        if a != b {
            self.lefts.swap(a, b);
            self.rights.swap(a, b);
            self.reverse.swap(a, b);
        }
    }

    /// fastutil's `Arrays.med3`.
    fn med3(&self, a: usize, b: usize, c: usize) -> usize {
        use std::cmp::Ordering::*;
        let ab = self.compare(a, b);
        let ac = self.compare(a, c);
        let bc = self.compare(b, c);
        if ab == Less {
            if bc == Less {
                b
            } else if ac == Less {
                c
            } else {
                a
            }
        } else if bc == Greater {
            b
        } else if ac == Greater {
            c
        } else {
            a
        }
    }

    /// fastutil's `swap(swapper, a, b, n)`.
    fn swap_n(&mut self, mut a: usize, mut b: usize, n: usize) {
        for _ in 0..n {
            self.swap(a, b);
            a += 1;
            b += 1;
        }
    }

    /// fastutil 8.5.18's `Arrays.quickSort(from, to, IntComparator, Swapper)`: insertion
    /// sort below 16, a pseudo-median of 9 above 128, three-way partitioning.
    fn quick_sort(&mut self, from: usize, to: usize) {
        use std::cmp::Ordering::*;
        let len = to - from;
        if len < 16 {
            for i in from..to {
                let mut j = i;
                while j > from && self.compare(j - 1, j) == Greater {
                    self.swap(j, j - 1);
                    j -= 1;
                }
            }
            return;
        }
        let mut m = from + len / 2;
        let mut l = from;
        let mut n = to - 1;
        if len > 128 {
            let s = len / 8;
            l = self.med3(l, l + s, l + 2 * s);
            m = self.med3(m - s, m, m + s);
            n = self.med3(n - 2 * s, n - s, n);
        }
        m = self.med3(l, m, n);
        // (Signed: `c` and `d` may step below `from`.)
        let (mut a, mut b, mut c, mut d) = (
            from as isize,
            from as isize,
            to as isize - 1,
            to as isize - 1,
        );
        loop {
            while b <= c {
                let comparison = self.compare(b as usize, m);
                if comparison == Greater {
                    break;
                }
                if comparison == Equal {
                    if a as usize == m {
                        m = b as usize;
                    } else if b as usize == m {
                        m = a as usize;
                    }
                    self.swap(a as usize, b as usize);
                    a += 1;
                }
                b += 1;
            }
            while c >= b {
                let comparison = self.compare(c as usize, m);
                if comparison == Less {
                    break;
                }
                if comparison == Equal {
                    if c as usize == m {
                        m = d as usize;
                    } else if d as usize == m {
                        m = c as usize;
                    }
                    self.swap(c as usize, d as usize);
                    d -= 1;
                }
                c -= 1;
            }
            if b > c {
                break;
            }
            if b as usize == m {
                m = d as usize;
            } else if c as usize == m {
                m = c as usize;
            }
            self.swap(b as usize, c as usize);
            b += 1;
            c -= 1;
        }
        let (from_i, to_i) = (from as isize, to as isize);
        let s = (a - from_i).min(b - a);
        self.swap_n(from, (b - s) as usize, s as usize);
        let s = (d - c).min(to_i - d - 1);
        self.swap_n(b as usize, (to_i - s) as usize, s as usize);
        let s = b - a;
        if s > 1 {
            self.quick_sort(from, from + s as usize);
        }
        let s = d - c;
        if s > 1 {
            self.quick_sort(to - s as usize, to);
        }
    }
}

impl SuffixArray {
    pub fn new() -> SuffixArray {
        SuffixArray::default()
    }

    /// `add`: one of an element's strings.
    pub fn add(&mut self, element: usize, text: &[u16]) {
        self.max_string_length = self.max_string_length.max(text.len());
        let index = self.list.len();
        self.list.push(element);
        self.word_starts.push(self.chars.len());
        for (i, &c) in text.iter().enumerate() {
            self.suffix_to_t.push(index);
            self.offsets.push(i);
            self.chars.push(i32::from(c));
        }
        self.suffix_to_t.push(index);
        self.offsets.push(text.len());
        self.chars.push(-1);
    }

    /// `generate`: the suffixes sorted by their first characters, doubling while twice
    /// the count is below the longest string (so not always by all of them).
    pub fn generate(&mut self) {
        let char_count = self.chars.len();
        let mut positions = self.chars.clone();
        let mut s = Sorting {
            lefts: vec![0; char_count],
            rights: vec![0; char_count],
            reverse: vec![0; char_count],
        };
        let max = char_count.min(self.max_string_length);
        let mut count = 1;
        while count * 2 < max {
            for i in 0..char_count {
                s.lefts[i] = positions[i];
                s.rights[i] = if i + count < char_count {
                    positions[i + count]
                } else {
                    -2
                };
                s.reverse[i] = i;
            }
            s.quick_sort(0, char_count);
            for i in 0..char_count {
                if i > 0 && s.lefts[i] == s.lefts[i - 1] && s.rights[i] == s.rights[i - 1] {
                    positions[s.reverse[i]] = positions[s.reverse[i - 1]];
                } else {
                    positions[s.reverse[i]] = i as i32;
                }
            }
            count *= 2;
        }
        // (Never sorted, `reverse` is all zeros: vanilla's too.)
        self.suffix_to_t = s.reverse.iter().map(|&i| self.suffix_to_t[i]).collect();
        self.offsets = s.reverse.iter().map(|&i| self.offsets[i]).collect();
    }

    fn compare(&self, text: &[u16], index: usize) -> std::cmp::Ordering {
        use std::cmp::Ordering::*;
        let start = self.word_starts[self.suffix_to_t[index]];
        let offset = self.offsets[index];
        for (i, &c) in text.iter().enumerate() {
            let p = self.chars[start + offset + i];
            if p == -1 {
                return Greater;
            }
            let c2 = p as u16;
            if c < c2 {
                return Less;
            }
            if c > c2 {
                return Greater;
            }
        }
        Equal
    }

    /// `search`: the elements with a string that has `text` in it, as the array finds
    /// them, in their order.
    pub fn search(&self, text: &[u16]) -> Vec<usize> {
        use std::cmp::Ordering::*;
        let suffix_count = self.suffix_to_t.len();
        let (mut low, mut high) = (0, suffix_count);
        while low < high {
            let mid = low + (high - low) / 2;
            if self.compare(text, mid) == Greater {
                low = mid + 1;
            } else {
                high = mid;
            }
        }
        if low >= suffix_count {
            return Vec::new();
        }
        let lower = low;
        high = suffix_count;
        while low < high {
            let mid = low + (high - low) / 2;
            if self.compare(text, mid) != Less {
                low = mid + 1;
            } else {
                high = mid;
            }
        }
        let mut adds: Vec<usize> = self.suffix_to_t[lower..low].to_vec();
        adds.sort_unstable();
        adds.dedup();
        let mut out: Vec<usize> = Vec::with_capacity(adds.len());
        for t in adds {
            let e = self.list[t];
            if !out.contains(&e) {
                out.push(e);
            }
        }
        out
    }
}

/// UTF-16 units of `s` as `toLowerCase(Locale.ROOT)` makes it.
pub fn lower(s: &str) -> Vec<u16> {
    s.to_lowercase().encode_utf16().collect()
}

/// `IdentifierSearchTree`: namespaces and paths, each in a suffix array.
#[derive(Debug, Clone, Default)]
pub struct IdentifierTree {
    namespaces: SuffixArray,
    paths: SuffixArray,
    empty: bool,
}

impl IdentifierTree {
    /// `IdentifierSearchTree.create`: each element's identifiers (`namespace:path`).
    pub fn new<'a>(ids: impl IntoIterator<Item = Vec<&'a str>>) -> IdentifierTree {
        let mut t = IdentifierTree {
            empty: true,
            ..IdentifierTree::default()
        };
        for (e, list) in ids.into_iter().enumerate() {
            t.empty = false;
            for id in list {
                let (ns, path) = id.split_once(':').unwrap_or(("minecraft", id));
                t.namespaces.add(e, &lower(ns));
                t.paths.add(e, &lower(path));
            }
        }
        if !t.empty {
            t.namespaces.generate();
            t.paths.generate();
        }
        t
    }

    pub fn search_namespace(&self, ns: &[u16]) -> Vec<usize> {
        if self.empty {
            return Vec::new();
        }
        self.namespaces.search(ns)
    }

    pub fn search_path(&self, path: &[u16]) -> Vec<usize> {
        if self.empty {
            return Vec::new();
        }
        self.paths.search(path)
    }
}

/// `IntersectionIterator` over two lists in element order.
fn intersection(a: &[usize], b: &[usize]) -> Vec<usize> {
    let (mut i, mut j, mut out) = (0, 0, Vec::new());
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Equal => {
                out.push(a[i]);
                i += 1;
                j += 1;
            }
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
        }
    }
    out
}

/// `MergingUniqueIterator` over two lists in element order.
fn merging_unique(a: &[usize], b: &[usize]) -> Vec<usize> {
    let (mut i, mut j, mut out) = (0, 0, Vec::new());
    loop {
        match (a.get(i), b.get(j)) {
            (None, None) => return out,
            (Some(&x), None) => {
                out.push(x);
                i += 1;
            }
            (None, Some(&y)) => {
                out.push(y);
                j += 1;
            }
            (Some(&x), Some(&y)) => {
                if x == y {
                    j += 1;
                }
                if x <= y {
                    out.push(x);
                    i += 1;
                } else {
                    out.push(y);
                    j += 1;
                }
            }
        }
    }
}

/// Java's `String.trim`: what is at or below a space goes from both ends.
fn java_trim(s: &[u16]) -> &[u16] {
    let start = s.iter().position(|&c| c > 0x20).unwrap_or(s.len());
    let end = s.iter().rposition(|&c| c > 0x20).map_or(start, |e| e + 1);
    &s[start..end.max(start)]
}

/// Splits a search at its first colon (`IdSearchTree.search`): the namespace and the
/// path, each trimmed.
fn split_colon(text: &[u16]) -> Option<(&[u16], &[u16])> {
    let colon = text.iter().position(|&c| c == u16::from(b':'))?;
    Some((java_trim(&text[..colon]), java_trim(&text[colon + 1..])))
}

/// `IdSearchTree` (the creative tag search).
#[derive(Debug, Clone, Default)]
pub struct IdTree {
    ids: IdentifierTree,
}

impl IdTree {
    pub fn new<'a>(ids: impl IntoIterator<Item = Vec<&'a str>>) -> IdTree {
        IdTree {
            ids: IdentifierTree::new(ids),
        }
    }

    /// `search` with the text already lower case.
    pub fn search(&self, text: &[u16]) -> Vec<usize> {
        match split_colon(text) {
            None => self.ids.search_path(text),
            Some((ns, path)) => {
                intersection(&self.ids.search_namespace(ns), &self.ids.search_path(path))
            }
        }
    }
}

/// `FullTextSearchTree` (the creative name search): each element's lines and its
/// identifiers.
#[derive(Debug, Clone, Default)]
pub struct FullTextTree {
    ids: IdentifierTree,
    plain: Option<SuffixArray>,
}

impl FullTextTree {
    /// `names`: each element's lines, already lower case (`SearchTree.plainText`
    /// lowers them); `ids`: each element's identifiers.
    pub fn new<'a>(
        names: &[Vec<Vec<u16>>],
        ids: impl IntoIterator<Item = Vec<&'a str>>,
    ) -> FullTextTree {
        let plain = (!names.is_empty()).then(|| {
            let mut a = SuffixArray::new();
            for (e, lines) in names.iter().enumerate() {
                for l in lines {
                    a.add(e, l);
                }
            }
            a.generate();
            a
        });
        FullTextTree {
            ids: IdentifierTree::new(ids),
            plain,
        }
    }

    fn plain(&self, text: &[u16]) -> Vec<usize> {
        self.plain
            .as_ref()
            .map_or_else(Vec::new, |p| p.search(text))
    }

    /// `search` with the text already lower case.
    pub fn search(&self, text: &[u16]) -> Vec<usize> {
        match split_colon(text) {
            None => self.plain(text),
            Some((ns, path)) => {
                let namespaces = self.ids.search_namespace(ns);
                let paths = self.ids.search_path(path);
                let names = self.plain(path);
                intersection(&namespaces, &merging_unique(&paths, &names))
            }
        }
    }
}

/// A tooltip line as the name search keeps it (`SessionSearchTrees.getTooltipLines`):
/// formatting codes taken out (`ChatFormatting.stripFormatting`), trimmed, lower case;
/// `None` for a line left empty.
pub fn search_line(line: &str) -> Option<Vec<u16>> {
    let mut stripped = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{a7}'
            && let Some(&n) = chars.peek()
            && matches!(n.to_ascii_lowercase(), '0'..='9' | 'a'..='f' | 'k'..='o' | 'r')
        {
            chars.next();
            continue;
        }
        stripped.push(c);
    }
    let units: Vec<u16> = stripped.encode_utf16().collect();
    let trimmed = java_trim(&units);
    if trimmed.is_empty() {
        return None;
    }
    Some(lower(&String::from_utf16_lossy(trimmed)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(strings: &[&str]) -> SuffixArray {
        let mut a = SuffixArray::new();
        for (i, s) in strings.iter().enumerate() {
            a.add(i, &lower(s));
        }
        a.generate();
        a
    }

    #[test]
    fn a_search_finds_what_has_the_text_in_it() {
        let a = tree(&["Oak Planks", "Spruce Planks", "Stone", "Oak Log", "Sponge"]);
        assert_eq!(a.search(&lower("oak")), [0, 3]);
        assert_eq!(a.search(&lower("planks")), [0, 1]);
        assert_eq!(a.search(&lower("sp")), [1, 4]);
        assert_eq!(a.search(&lower("o")), [0, 2, 3, 4]);
        assert!(a.search(&lower("dirt")).is_empty());
    }

    #[test]
    fn an_identifier_search_needs_both_halves() {
        let t = FullTextTree::new(
            &[vec![lower("stone")], vec![lower("granite")]],
            [vec!["minecraft:stone"], vec!["other:granite"]],
        );
        assert_eq!(t.search(&lower("minecraft:")), [0]);
        assert_eq!(t.search(&lower("other: gran")), [1]);
        assert_eq!(t.search(&lower(":stone")), [0]);
    }

    #[test]
    fn lines_lose_their_formatting_codes_and_spaces() {
        assert_eq!(
            search_line(" \u{a7}aGreen \u{a7}lBold "),
            Some(lower("Green Bold"))
        );
        assert_eq!(search_line("   "), None);
        assert_eq!(search_line("\u{a7}zx"), Some(lower("\u{a7}zx")));
    }
}

#[cfg(test)]
mod golden_tests {
    use super::*;

    use super::super::search_golden as golden;

    struct Lcg(u64);

    impl Lcg {
        fn next(&mut self, n: usize) -> usize {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((self.0 >> 33) % n as u64) as usize
        }
    }

    fn fnv(s: &str) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for b in s.bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
        h
    }

    /// SearchGolden's `query`.
    fn query(r: &mut Lcg, lines: &[Vec<String>]) -> String {
        let kind = r.next(10);
        let l = &lines[r.next(lines.len())];
        let s = &l[r.next(l.len())];
        let start = r.next(s.len());
        let len = 1 + r.next(14.min(s.len() - start));
        let part = &s[start..start + len];
        match kind {
            0 => format!("minecraft:{part}"),
            1 => format!("{part}:{}", &l[0][..3.min(l[0].len())]),
            2 => format!(" c : {part}"),
            3 => {
                let a = (b'a' + r.next(26) as u8) as char;
                let b = (b'a' + r.next(26) as u8) as char;
                format!("{a}{b}")
            }
            _ => part.to_owned(),
        }
    }

    /// 3000 queries through vanilla's `FullTextSearchTree` and `IdSearchTree` over the
    /// item registry's names (SearchGolden): the same elements, in the same order.
    #[test]
    fn searches_find_what_vanillas_trees_find() {
        use ruthenium_data::v26_3::registries::Item;
        let keys: Vec<&str> = (0..)
            .map_while(|i| Item::from_id(i).map(|it| it.name()))
            .collect();
        let n = keys.len();
        let path = |k: &str| k[k.find(':').unwrap() + 1..].replace('_', " ");
        let lines: Vec<Vec<String>> = (0..n)
            .map(|i| {
                let mut l = vec![path(keys[i])];
                if i % 7 == 0 {
                    let more: Vec<String> = (1..=3).map(|k| path(keys[(i + k) % n])).collect();
                    l.push(more.join(" "));
                }
                if i % 11 == 0 {
                    l.push(format!("level {}", i % 5));
                }
                l
            })
            .collect();
        let names: Vec<Vec<Vec<u16>>> = lines
            .iter()
            .map(|l| l.iter().map(|s| lower(s)).collect())
            .collect();
        let tree = FullTextTree::new(&names, keys.iter().map(|k| vec![*k]));
        let tag_ids: Vec<[String; 2]> = (0..n)
            .map(|i| {
                let t = if i % 13 == 0 {
                    "logs".to_owned()
                } else {
                    format!("tag_{}", i % 17)
                };
                let c = if i % 5 == 0 {
                    "ores".to_owned()
                } else {
                    format!("x{}", i % 3)
                };
                [format!("minecraft:{t}"), format!("c:{c}")]
            })
            .collect();
        let tags = IdTree::new(tag_ids.iter().map(|t| vec![t[0].as_str(), t[1].as_str()]));
        let mut r = Lcg(0x5EA4_C400);
        let mut bad = 0;
        for (q, &(want_names, want_tags)) in golden::QUERIES.iter().enumerate() {
            let text = query(&mut r, &lines);
            let list =
                |found: Vec<usize>| -> String { found.iter().map(|i| format!("{i},")).collect() };
            let a = fnv(&format!("{text}|{}", list(tree.search(&lower(&text)))));
            let b = fnv(&format!("{text}|{}", list(tags.search(&lower(&text)))));
            if a != want_names || b != want_tags {
                if bad < 5 {
                    eprintln!(
                        "query {q} {text:?} differs (names {}, tags {})",
                        a != want_names,
                        b != want_tags
                    );
                }
                bad += 1;
            }
        }
        assert_eq!(bad, 0, "{bad} of {} queries differ", golden::QUERIES.len());
    }

    /// 2000 queries over lines sharing long prefixes, many alike (SearchGolden): where
    /// vanilla's array is sorted only as deep as its doubling went and its partition's
    /// pivot moves, the same elements found.
    #[test]
    fn searches_over_long_prefixes_find_what_vanillas_find() {
        let mut r = Lcg(0x5EA4_C401);
        let lines: Vec<Vec<String>> = (0..400)
            .map(|_| {
                // (SearchGolden draws the count again at every turn of its loop.)
                let mut l = Vec::new();
                while l.len() < 1 + r.next(2) {
                    let mut s = "q".repeat(12 + r.next(6));
                    for _ in 0..r.next(6) {
                        s.push(if r.next(2) == 0 { 'a' } else { 'b' });
                    }
                    l.push(s);
                }
                l
            })
            .collect();
        let names: Vec<Vec<Vec<u16>>> = lines
            .iter()
            .map(|l| l.iter().map(|s| lower(s)).collect())
            .collect();
        let keys: Vec<String> = (0..400).map(|i| format!("minecraft:e{i}")).collect();
        let tree = FullTextTree::new(&names, keys.iter().map(|k| vec![k.as_str()]));
        let mut bad = 0;
        for &want in golden::PREFIXED {
            let mut q = "q".repeat(r.next(20));
            for _ in 0..r.next(5) {
                q.push(if r.next(2) == 0 { 'a' } else { 'b' });
            }
            let found: String = tree
                .search(&lower(&q))
                .iter()
                .map(|i| format!("{i},"))
                .collect();
            if fnv(&format!("{q}|{found}")) != want {
                bad += 1;
            }
        }
        assert_eq!(bad, 0, "{bad} of {} queries differ", golden::PREFIXED.len());
    }
}
