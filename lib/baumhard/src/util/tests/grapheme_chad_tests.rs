// SPDX-License-Identifier: MPL-2.0

use lazy_static::lazy_static;

use crate::util::grapheme_chad::{
    count_grapheme_clusters, count_number_lines, delete_back_unicode, delete_front_unicode,
    delete_grapheme_at, find_byte_index_of_grapheme, find_nth_line_byte_range, find_nth_line_grapheme_range,
    grapheme_display_width, insert_new_lines, insert_str_at_grapheme, insert_str_at_grapheme_counted,
    push_spaces, replace_graphemes_until_newline, scalar_display_width, slice_to_newline,
    split_off_graphemes, truncate_to_display_width, word_left, word_right,
};

lazy_static! {
    pub static ref SPLIT_GRAPHEMES_TEST: Vec<(&'static str, usize, &'static str, &'static str)> = vec![
        ("abcd🍕1234", 4, "abcd", "🍕1234"),
        ("abcd🍕1234", 5, "abcd🍕", "1234"),
        ("abcd🙏🏻1234", 5, "abcd🙏🏻", "1234"),
        ("abcd🙏🏻1234", 4, "abcd", "🙏🏻1234"),
        ("🙏🏻🙏🏻🙏🏻🙏🏻1234", 4, "🙏🏻🙏🏻🙏🏻🙏🏻", "1234"),
        (
            "🙏🏻🙏🏻🙏🏻🙏🏻🍕🍕🍕🍕",
            3,
            "🙏🏻🙏🏻🙏🏻",
            "🙏🏻🍕🍕🍕🍕"
        ),
        ("🏳️‍🌈👩‍👧‍👦👯‍♂️👰‍♂️👨‍🚀", 3, "🏳️‍🌈👩‍👧‍👦👯‍♂️", "👰‍♂️👨‍🚀"),
        ("", 2, "", ""),
    ];
    pub static ref REMOVE_PREFIX_TESTS: Vec<(&'static str, usize, &'static str)> = vec![
        // n = 0 must be a no-op, never eating a leading grapheme — regardless of
        // whether that grapheme is ASCII, a multi-scalar emoji, or a ZWJ cluster.
        ("abcd", 0, "abcd"),
        ("🙏🏻abcd", 0, "🙏🏻abcd"),
        ("👨‍👩‍👧abcd", 0, "👨‍👩‍👧abcd"),
        ("", 0, ""),
        ("abcd🍕1234", 3, "d🍕1234"),
        ("\n\nabcd🍕1234", 3, "bcd🍕1234"),
        ("\n\n bcd🍕1234", 3, "bcd🍕1234"),
        ("abcd🍕1234", 4, "🍕1234"),
        ("abcd🍕1234", 5, "1234"),
        ("abcd🍕1234", 6, "234"),
        ("abcd🍕1234", 7, "34"),
        ("abcd🍕1234", 8, "4"),
        ("abcd🍕1234", 9, ""),
        ("abcd🍕1234", 10, ""),
        ("🙏🏻🙏🏻🙏🏻", 1, "🙏🏻🙏🏻"),
        ("🙏🏻🙏🏻🙏🏻", 2, "🙏🏻"),
        ("🍕🍕🍕🍕🍕🍕🍕🍕🍕🍕", 2, "🍕🍕🍕🍕🍕🍕🍕🍕"),
    ];
    pub static ref TRUNCATE_TESTS: Vec<(&'static str, usize, &'static str)> = vec![
        // n = 0 is a no-op on the tail too — the symmetric partner of the
        // delete_front_unicode(_, 0) no-op above.
        ("abcd", 0, "abcd"),
        ("abcd🙏🏻", 0, "abcd🙏🏻"),
        ("", 0, ""),
        ("abcd🍕1234", 3, "abcd🍕1"),
        ("abcd🍕1234", 4, "abcd🍕"),
        ("abcd🍕1234", 5, "abcd"),
        ("abcd🍕1234", 6, "abc"),
        ("abcd🍕1234", 7, "ab"),
        ("abcd🍕1234", 8, "a"),
        ("abcd🍕1234", 9, ""),
        ("abcd🍕1234", 10, ""),
        ("🙏🏻🙏🏻🙏🏻", 1, "🙏🏻🙏🏻"),
        ("🙏🏻🙏🏻🙏🏻", 2, "🙏🏻"),
        ("🍕🍕🍕🍕🍕🍕🍕🍕🍕🍕", 2, "🍕🍕🍕🍕🍕🍕🍕🍕"),
        ("🍕🍕🍕🍕🍕🍕🍕🍕🍕🍕\n", 2, "🍕🍕🍕🍕🍕🍕🍕🍕🍕"),
        (
            "🍕🍕🍕🍕🍕🍕🍕🍕🍕🍕\n\n\n\n\n\n\n\n\n\n\n",
            12,
            "🍕🍕🍕🍕🍕🍕🍕🍕🍕"
        ),
    ];
    pub static ref COUNT_LINES_TEST: Vec<(&'static str, usize)> = vec![
        ("abcd🍕1234", 1),
        ("abcd\n", 2),
        ("abcd\n\n", 3),
        ("\n\n", 3),
        ("", 1),
        ("\n", 2),
        ("\n\n\n", 4),
        ("\nho\nhi\nhello", 4),
        ("\n🙏🏻\n🙏🏻🙏🏻\n🙏🏻🙏🏻🙏🏻\n🙏🏻\n🙏🏻\n\n\n\n\n\n\n\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n\n\n\n\n\n🙏🏻🙏🏻🙏🏻\n\n\n\n", 24),
        ("\nho\nhi\nhello🙏🏻🙏🏻🙏🏻\n\n\n🙏🏻\n\n\n🙏🏻\n\n\n🙏🏻\n\n\n\n\n🙏🏻\n\n\n\n🙏🏻\n\n", 24),
    ];

    pub static ref NTH_LINE_BYTE_INDICES_TEST: Vec<(&'static str, usize, Option<(usize, usize)>)> = vec![
        ("\n", 0, Some((0, 0))),
        ("", 0, None),
        ("a", 0, Some((0,1))),
        ("a\n", 1, None),
        ("a\n", 0, Some((0,1))),
        ("", 1, None),
        ("Hello\nxxxxxxxxxxqqqqqqqqqqxxxxxxxxxxqqqqqqqqqq\n", 1, Some((6, 46))),
        ("Hello\nxxxxxxxxxxqqqqqqqqqqxxxxxxxxxxqqqqqqqqqq", 1, Some((6, 46))),
        ("\n\n", 1, Some((1, 1))),
        ("\nhi\n", 1, Some((1, 3))),
        ("\nhi\n", 2, None),
        ("\nhi\na\nb\nc\nd", 4, Some((8, 9))),
        ("\n🙏🏻\n🙏🏻\n", 2, Some((1+8+1, 1+8+1+8))),
        ("🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻", 1, Some((8*10+1, (8*10+1)+8*10))),
        ("🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻", 2, Some((20*8 + 2, (20*8 + 2) + 10*8))),
    ];

    pub static ref NTH_LINE_GRAPHEME_INDICES_TEST: Vec<(&'static str, usize, Option<(usize, usize)>)> = vec![
        ("\n", 0, Some((0, 0))),
        ("", 0, None),
        ("a", 0, Some((0,1))),
        ("a\n", 1, None),
        ("a\n", 0, Some((0,1))),
        ("", 1, None),
        ("Hello\nxxxxxxxxxxqqqqqqqqqqxxxxxxxxxxqqqqqqqqqq\n", 1, Some((6, 46))),
        ("\n🙏🏻\n🙏🏻\n", 2, Some((3, 4))),
        ("\n🙏🏻\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n", 2, Some((3, 8))),
        ("\n🙏🏻🙏🏻\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n", 2, Some((4, 9))),
        ("🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻", 0, Some((0, 10))),
        ("🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻", 1, Some((11, 21))),
        ("🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻", 2, Some((22, 32))),
        ("🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻", 1, Some((11, 11))),
        ("🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\na\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻", 1, Some((11, 12))),
        ("🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻", 2, Some((12, 22))),
    ];

    pub static ref REPLACE_GRAPHEMES_UNTIL_NEWLINE_TEST: Vec<(&'static str, usize, &'static str, &'static str)> = vec![
        ("abc", 0, "abc", "abc"),
        ("abd", 0, "abc", "abd"),
        ("abd", 0, "", "abd"),
        ("abd", 0, "abcd", "abdd"),
        ("🙏🏻🙏🏻🙏🏻", 0, "", "🙏🏻🙏🏻🙏🏻"),
        ("🙏🏻🙏🏻🙏🏻", 0, "\n", "🙏🏻🙏🏻🙏🏻\n"),
        ("🙏🏻", 0, "abcd", "🙏🏻bcd"),
        ("a", 0, "🙏🏻bcd", "abcd"),
        ("aaaaaaaaaaaaaa", 0, "🙏🏻bcd12🙏🏻3456789", "aaaaaaaaaaaaaa"),
        ("aaaaaaaaaaaaaa", 0, "🙏🏻bcd12🙏🏻\n3456789", "aaaaaaaaaaaaaa\n3456789"),
        ("🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻", 0,
            "🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻",
            "🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻"),
        ("12345", 10, "01234\n678\n", "01234\n678\n12345"),
        ("12345", 10, "01234\n678\n\n", "01234\n678\n12345\n"),
        ("12345", 10, "01234\n678\nabcde\n", "01234\n678\n12345\n"),
        ("12345", 10, "01234\n678\n     \n", "01234\n678\n12345\n"),
        ("123", 10, "01234\n678\n     \n", "01234\n678\n123  \n"),
        ("123", 10, "01234\n678\nabcde\n", "01234\n678\n123de\n"),
        ("🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻", 11, "🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n", "🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻"),
        ("@@@@@@@@@@", 63,
            "🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@\n          🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n          🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n          🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻",
            "🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@\n@@@@@@@@@@🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n          🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n          🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻"),
    ];

    pub static ref COUNT_GRAPHEMES_TEST: Vec<(&'static str, usize)> = vec![
        ("abcde", 5),
        ("🙏🏻", 1),
        ("abcd🙏🏻", 5),
        ("🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻", 5),
        ("🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻", 10),

    ];
    pub static ref COMPLEX_NEWLINE_STRING: &'static str =
    "🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@\n          🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n          🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n          🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻";

    pub static ref INDEX_OF_GRAPHEME_TEST: Vec<(&'static str, usize, Option<usize>)> = vec![
        ("abcde", 4, Some(4)),
        ("🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻", 4, Some(4*8)),
        ("🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@\n", 10, Some(10*8)),
        ("🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@\n", 11, Some(10*8 + 1)),
        ("🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@\n", 20, Some(10*8 + 10)),
        ("🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@\n", 21, None),
        ("🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@\n", 10, Some(10*8)),
        ("🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@\n", 21, Some(10*8 + 11)),
        ("🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@\n", 31, Some(20*8 + 11)),
        ("🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@\n", 41, Some(20*8 + 21)),
        ("🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@\n", 42, None),
        (COMPLEX_NEWLINE_STRING.clone(), 10, Some(10*8)),
        (COMPLEX_NEWLINE_STRING.clone(), 11, Some(10*8 + 1)),
        (COMPLEX_NEWLINE_STRING.clone(), 20, Some(10*8 + 10)),
        (COMPLEX_NEWLINE_STRING.clone(), 21, Some(10*8 + 11)),
        (COMPLEX_NEWLINE_STRING.clone(), 31, Some(20*8 + 11)),
        (COMPLEX_NEWLINE_STRING.clone(), 41, Some(20*8 + 21)),
        (COMPLEX_NEWLINE_STRING.clone(), 41, Some(20*8 + 21)),
        (COMPLEX_NEWLINE_STRING.clone(), 52, Some(30*8 + 22)),
        (COMPLEX_NEWLINE_STRING.clone(), 62, Some(30*8 + 32)),
        (COMPLEX_NEWLINE_STRING.clone(), 63, Some(30*8 + 33)),
        (COMPLEX_NEWLINE_STRING.clone(), 83, Some(40*8 + 43)),
        (COMPLEX_NEWLINE_STRING.clone(), 84, Some(40*8 + 44)),
        (COMPLEX_NEWLINE_STRING.clone(), 94, Some(40*8 + 54)),
        (COMPLEX_NEWLINE_STRING.clone(), 104, Some(50*8 + 54)),
        (COMPLEX_NEWLINE_STRING.clone(), 105, Some(50*8 + 55)),
        (COMPLEX_NEWLINE_STRING.clone(), 115, Some(50*8 + 65)),
        (COMPLEX_NEWLINE_STRING.clone(), 124, Some(59*8 + 65)),
        // This is a little confusing, there's 60 "hands emojis" but the last one doesn't count here
        // Since we can only get a byte index to its beginning, its bytes aren't included in the count
    ];
    pub static ref SLICE_TO_NEWLINE_TEST: Vec<(&'static str, usize, &'static str)> = vec![
        ("abcde\n", 0, "abcde"),
        ("🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻", 0, "🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻"),
        ("🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻", 5*8+1, "🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻"),
        (COMPLEX_NEWLINE_STRING.clone(),
            0, "🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@"),
        (COMPLEX_NEWLINE_STRING.clone(),
            find_byte_index_of_grapheme(&COMPLEX_NEWLINE_STRING, 21).unwrap(), "🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@"),
    ];
}

#[test]
pub fn test_slice_to_newline() {
    do_slice_to_newline();
}

pub fn do_slice_to_newline() {
    for (subject, index, expected) in SLICE_TO_NEWLINE_TEST.clone() {
        let result = slice_to_newline(subject, index);
        assert_eq!(result, expected);
    }
}

#[test]
pub fn test_split_graphemes() {
    do_split_graphemes();
}

pub fn do_split_graphemes() {
    for (original, split_at, remainder, new) in SPLIT_GRAPHEMES_TEST.clone() {
        let mut actual_remainder = original.to_string();
        let actual_new = split_off_graphemes(&mut actual_remainder, split_at);
        assert_eq!(actual_remainder, remainder);
        assert_eq!(actual_new, new);
    }
}

#[test]
pub fn test_find_byte_index_of_grapheme() {
    do_find_byte_index_of_grapheme();
}

pub fn do_find_byte_index_of_grapheme() {
    for (string, graph_index, expected_byte_index) in INDEX_OF_GRAPHEME_TEST.clone() {
        if expected_byte_index.is_some() && expected_byte_index.clone().unwrap() >= string.len() {
            panic!("Expected byte index is out of bounds!! This is a test error.");
        }
        let result = find_byte_index_of_grapheme(string, graph_index);
        assert_eq!(result, expected_byte_index);
    }
}

#[test]
pub fn test_replace_graphemes_until_newline() {
    do_replace_graphemes_until_newline();
}

pub fn do_replace_graphemes_until_newline() {
    for (source, idx, target, expected) in REPLACE_GRAPHEMES_UNTIL_NEWLINE_TEST.clone() {
        let mut result = target.to_string();
        replace_graphemes_until_newline(&mut result, idx, &source);
        assert_eq!(result, expected);
    }
}

#[test]
pub fn test_count_grapheme_clusters() {
    do_count_grapheme_clusters();
}

pub fn do_count_grapheme_clusters() {
    for (string, num_clusters) in COUNT_GRAPHEMES_TEST.clone() {
        let result = count_grapheme_clusters(string);
        assert_eq!(result, num_clusters);
    }
}

#[test]
pub fn test_find_nth_line_byte_indices() {
    do_find_nth_line_byte_indices();
}

pub fn do_find_nth_line_byte_indices() {
    for (str, n, idx) in NTH_LINE_BYTE_INDICES_TEST.clone() {
        let result = find_nth_line_byte_range(str, n);
        assert_eq!(result, idx);
    }
}

#[test]
pub fn test_find_nth_line_grapheme_indices() {
    do_find_nth_line_grapheme_indices();
}

pub fn do_find_nth_line_grapheme_indices() {
    for (str, n, idx) in NTH_LINE_GRAPHEME_INDICES_TEST.clone() {
        let result = find_nth_line_grapheme_range(str, n);
        assert_eq!(result, idx);
    }
}

#[test]
pub fn test_remove_prefix_unicode() {
    do_remove_prefix_unicode();
}

pub fn do_remove_prefix_unicode() {
    for (original, n, expected) in REMOVE_PREFIX_TESTS.clone() {
        let mut our_og = original.to_string();
        delete_front_unicode(&mut our_og, n);
        assert_eq!(our_og, expected);
    }
}

#[test]
pub fn test_insert_new_lines() {
    do_insert_new_lines();
}

pub fn do_insert_new_lines() {
    let mut my_string = String::new();
    insert_new_lines(&mut my_string, 4);
    let mut count = count_number_lines(&my_string);
    assert_eq!(count, 5);
    insert_new_lines(&mut my_string, 5);
    count = count_number_lines(&my_string);
    assert_eq!(count, 10);
    insert_new_lines(&mut my_string, 5000);
    count = count_number_lines(&my_string);
    assert_eq!(count, 5010);
}
#[test]
pub fn test_push_spaces() {
    do_push_spaces();
}

pub fn do_push_spaces() {
    let mut my_string = String::new();
    push_spaces(&mut my_string, 5);
    assert_eq!(my_string.len(), count_grapheme_clusters(&my_string));
    assert_eq!(my_string.len(), 5);
    push_spaces(&mut my_string, 15);
    assert_eq!(my_string.len(), count_grapheme_clusters(&my_string));
    assert_eq!(my_string.len(), 20);
    push_spaces(&mut my_string, 0);
    assert_eq!(my_string.len(), 20);
}

#[test]
pub fn test_count_number_of_lines() {
    do_count_grapheme_clusters();
}

pub fn do_count_number_of_lines() {
    for (str, num_lines) in COUNT_LINES_TEST.clone() {
        let result = count_number_lines(str);
        assert_eq!(result, num_lines);
    }
}

#[test]
pub fn test_truncate_unicode() {
    do_truncate_unicode();
}

pub fn do_truncate_unicode() {
    for (original, n, expected) in TRUNCATE_TESTS.clone() {
        let mut our_og = original.to_string();
        delete_back_unicode(&mut our_og, n);
        assert_eq!(our_og, expected);
    }
}

#[test]
pub fn test_insert_str_at_grapheme() {
    do_insert_str_at_grapheme();
}

pub fn do_insert_str_at_grapheme() {
    // ASCII insert in the middle.
    let mut s = String::from("abcd");
    insert_str_at_grapheme(&mut s, 2, "X");
    assert_eq!(s, "abXcd");

    // Insert past the end appends.
    let mut s = String::from("abc");
    insert_str_at_grapheme(&mut s, 99, "Z");
    assert_eq!(s, "abcZ");

    // Multi-byte char doesn't split a grapheme.
    let mut s = String::from("café"); // 4 graphemes; 'é' is 2 bytes
    insert_str_at_grapheme(&mut s, 4, "!");
    assert_eq!(s, "café!");
    let mut s = String::from("café");
    insert_str_at_grapheme(&mut s, 3, "!");
    assert_eq!(s, "caf!é");

    // Emoji ZWJ cluster is treated as one unit.
    let mut s = String::from("ab🧑‍🚀cd");
    insert_str_at_grapheme(&mut s, 3, "Z");
    assert_eq!(s, "ab🧑‍🚀Zcd");
}

#[test]
pub fn test_insert_str_at_grapheme_counted() {
    do_insert_str_at_grapheme_counted();
}

pub fn do_insert_str_at_grapheme_counted() {
    let mut s = String::new();
    let delta = insert_str_at_grapheme_counted(&mut s, 0, "abc");
    assert_eq!(s, "abc");
    assert_eq!(delta, 3);

    let mut s = String::new();
    let delta = insert_str_at_grapheme_counted(&mut s, 0, "e\u{0301}");
    assert_eq!(s, "e\u{0301}");
    assert_eq!(delta, 1);

    let mut s = String::from("e");
    let delta = insert_str_at_grapheme_counted(&mut s, 1, "\u{0301}");
    assert_eq!(s, "e\u{0301}");
    assert_eq!(delta, 0);

    let mut s = String::new();
    let delta = insert_str_at_grapheme_counted(&mut s, 0, "\u{1112}\u{1161}\u{11AB}");
    assert_eq!(s, "\u{1112}\u{1161}\u{11AB}");
    assert_eq!(delta, 1);

    let mut s = String::new();
    let family = "\u{1F469}\u{200D}\u{1F469}\u{200D}\u{1F466}";
    let delta = insert_str_at_grapheme_counted(&mut s, 0, family);
    assert_eq!(s, family);
    assert_eq!(delta, 1);
}

#[test]
pub fn test_grapheme_display_width() {
    do_grapheme_display_width();
}

pub fn do_grapheme_display_width() {
    // ASCII → one cell each.
    assert_eq!(grapheme_display_width(""), 0);
    assert_eq!(grapheme_display_width("abcd"), 4);
    assert_eq!(grapheme_display_width("❯ color"), 7);

    // Box-drawing characters (U+2500..) are ambiguous-width in Unicode
    // but render at one cell in the app's monospace chain. The table
    // deliberately keeps them at 1.
    assert_eq!(grapheme_display_width("╭─╮"), 3);
    assert_eq!(grapheme_display_width("│ │"), 3);

    // East-Asian-Wide: each CJK ideograph counts as two cells.
    assert_eq!(grapheme_display_width("日"), 2);
    assert_eq!(grapheme_display_width("日本語"), 6);

    // Hiragana / Katakana / Hangul.
    assert_eq!(grapheme_display_width("あい"), 4);
    assert_eq!(grapheme_display_width("가나"), 4);

    // Combining marks fold into their base: "café" as NFD (e + ´) is
    // still 4 cells because the combining acute is zero-width.
    assert_eq!(grapheme_display_width("cafe\u{0301}"), 4);

    // ZWJ emoji cluster (`🧑‍🚀`) has base 🧑 — outside the table's
    // wide ranges, so we return 1. This is a known under-measure for
    // terminal emoji; the app uses cosmic-text which renders the whole
    // ZWJ cluster at its own advance anyway, so the border-alignment
    // use-case is unaffected.
    assert_eq!(grapheme_display_width("🧑‍🚀"), 1);

    // Scalar-level spot checks.
    assert_eq!(scalar_display_width('a'), 1);
    assert_eq!(scalar_display_width('日'), 2);
    assert_eq!(scalar_display_width('\u{0301}'), 0); // combining acute
    assert_eq!(scalar_display_width('\u{200D}'), 0); // ZWJ
}

#[test]
pub fn test_truncate_to_display_width() {
    do_truncate_to_display_width();
}

pub fn do_truncate_to_display_width() {
    // ASCII: exact cell match.
    assert_eq!(truncate_to_display_width("abcdef", 3), "abc");
    assert_eq!(truncate_to_display_width("abcdef", 0), "");
    assert_eq!(truncate_to_display_width("abcdef", 100), "abcdef");

    // CJK: each char is 2 cells; an odd max cuts cleanly on the
    // grapheme boundary rather than mid-glyph.
    assert_eq!(truncate_to_display_width("日本語", 3), "日"); // 2+2>3 after "日"
    assert_eq!(truncate_to_display_width("日本語", 4), "日本");
    assert_eq!(truncate_to_display_width("日本語", 2), "日");
    assert_eq!(truncate_to_display_width("日本語", 0), "");

    // Mix: 3 ASCII + 1 CJK = 5 cells; trim to 4 drops the CJK.
    assert_eq!(truncate_to_display_width("abc日", 4), "abc");
    assert_eq!(truncate_to_display_width("abc日", 5), "abc日");

    // Combining marks: NFD "café" is 4 cells, e+́ folds into one cell.
    assert_eq!(truncate_to_display_width("cafe\u{0301}", 3), "caf");
    assert_eq!(truncate_to_display_width("cafe\u{0301}", 4), "cafe\u{0301}");
}

#[test]
pub fn test_delete_grapheme_at() {
    do_delete_grapheme_at();
}

pub fn do_delete_grapheme_at() {
    // ASCII delete in the middle.
    let mut s = String::from("abcd");
    delete_grapheme_at(&mut s, 1);
    assert_eq!(s, "acd");

    // Delete past the end is a no-op.
    let mut s = String::from("abc");
    delete_grapheme_at(&mut s, 99);
    assert_eq!(s, "abc");

    // Delete the last cluster.
    let mut s = String::from("abc");
    delete_grapheme_at(&mut s, 2);
    assert_eq!(s, "ab");

    // Multi-byte char and ZWJ cluster delete as one unit.
    let mut s = String::from("café");
    delete_grapheme_at(&mut s, 3);
    assert_eq!(s, "caf");
    let mut s = String::from("ab🧑‍🚀cd");
    delete_grapheme_at(&mut s, 2);
    assert_eq!(s, "abcd");
}

#[test]
pub fn test_word_left() {
    do_word_left();
}

pub fn do_word_left() {
    // Cursor at the end of a single word lands at the word's start.
    assert_eq!(word_left("hello", 5), 0);

    // Cursor at 0 is a no-op.
    assert_eq!(word_left("hello world", 0), 0);

    // Skip backwards past leading boundary chars then through the
    // preceding word — `"foo, bar"` cursor=8 (end) → 5 (start of
    // "bar"); cursor=5 (start of "bar") → 0 (start of "foo").
    assert_eq!(word_left("foo, bar", 8), 5);
    assert_eq!(word_left("foo, bar", 5), 0);

    // Walk through two words: `"foo bar"` cursor=7 → 4 → 0.
    assert_eq!(word_left("foo bar", 7), 4);
    assert_eq!(word_left("foo bar", 4), 0);

    // Empty string is a no-op for any cursor.
    assert_eq!(word_left("", 0), 0);
    assert_eq!(word_left("", 5), 0);

    // Cursor past the grapheme count is clamped to the count.
    assert_eq!(word_left("hello", 99), 0);

    // Single grapheme.
    assert_eq!(word_left("a", 1), 0);

    // Emoji is a non-word boundary — `"foo🍕bar"` cursor=7 → 4
    // (start of "bar" since 🍕 is one grapheme cluster between
    // graphemes 3 and 4).
    assert_eq!(word_left("foo🍕bar", 7), 4);

    // ZWJ cluster (👨‍👩‍👧) — first scalar `'\u{1F468}'` is non-
    // alphanumeric, so the entire cluster counts as one boundary
    // grapheme. `"foo👨‍👩‍👧bar"` cursor=7 → 4.
    assert_eq!(word_left("foo👨‍👩‍👧bar", 7), 4);

    // Combining-mark cluster (NFD `café` = "cafe\u{0301}") — the
    // base scalar is alphanumeric, so the cluster IS a word
    // grapheme. cursor=4 (end of "café") → 0.
    let cafe_nfd = "cafe\u{0301}";
    assert_eq!(word_left(cafe_nfd, 4), 0);

    // Regional-indicator pair (🇺🇸 = two scalars, one cluster) —
    // first scalar `'\u{1F1FA}'` is non-alphanumeric, cluster is
    // a boundary. `"foo🇺🇸bar"` cursor=7 → 4.
    assert_eq!(word_left("foo🇺🇸bar", 7), 4);
}

#[test]
pub fn test_word_right() {
    do_word_right();
}

pub fn do_word_right() {
    // Cursor at 0 of a single word lands past the word's end.
    assert_eq!(word_right("hello", 0), 5);

    // Cursor at end is a no-op.
    assert_eq!(word_right("hello", 5), 5);

    // Cursor past the end is clamped at the grapheme count.
    assert_eq!(word_right("hello", 99), 5);

    // Skip leading boundary chars then through the following word.
    assert_eq!(word_right(", foo bar", 0), 5);
    assert_eq!(word_right("foo bar", 0), 3);
    assert_eq!(word_right("foo bar", 3), 7);

    // Empty string is a no-op for any cursor.
    assert_eq!(word_right("", 0), 0);
    assert_eq!(word_right("", 5), 0);

    // Single grapheme.
    assert_eq!(word_right("a", 0), 1);

    // Emoji boundary — `"foo🍕bar"` cursor=0 → 3 (end of "foo");
    // cursor=3 → 7 (past the boundary 🍕 and through "bar").
    assert_eq!(word_right("foo🍕bar", 0), 3);
    assert_eq!(word_right("foo🍕bar", 3), 7);

    // ZWJ cluster — `"foo👨‍👩‍👧bar"` cursor=3 → 7.
    assert_eq!(word_right("foo👨‍👩‍👧bar", 3), 7);

    // Combining-mark cluster — base scalar alphanumeric, so the
    // cluster IS a word grapheme. cursor=0 of `"café"` (NFD) → 4.
    let cafe_nfd = "cafe\u{0301}";
    assert_eq!(word_right(cafe_nfd, 0), 4);

    // Regional-indicator pair — `"foo🇺🇸bar"` cursor=3 → 7.
    assert_eq!(word_right("foo🇺🇸bar", 3), 7);
}
