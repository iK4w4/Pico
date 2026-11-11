//! 0.32 download manager: the pure core engine
//! (`core::download`) — segment planning, work-stealing,
//! filename/cookie safety, the resume decision, persisted
//! task state, and the speed/ETA helpers.
//!
//! Everything here is pure or temp-dir I/O: no network, no
//! window. The transport and manager layers are exercised
//! elsewhere; this suite pins the CONTRACT in the 0.32 brief.

use pico_core::core::download::{
    classify_failure, cookie_capture_allowed, cookie_expiry_unix_ms, cookie_header,
    dedupe_file_name, delete_task_file, eta_seconds, eta_text, failure_hint,
    file_name_from_disposition, file_name_from_url, http_status_of, is_downloadable_url,
    load_tasks, parse_content_range, percent, plan_segments, refresh_decision,
    resolve_downloads_dir, resume_decision, retry_policy, sanitize_cookies, sanitize_file_name,
    save_task, speed_text, strip_url_userinfo, triage_download_uri, url_host, Claim, CookiePair,
    DownloadTriage, FailureClass, GenerationGate, Planner, ProbeFacts, RefreshDecision,
    ResumeDecision, RetryPolicy, Segment, SpeedMeter, TaskState, TaskStatus, TokenBucket,
    AUTO_RETRY_BACKOFF_MS, AUTO_RETRY_MAX, DEFAULT_CONNECTIONS, MAX_CONNECTIONS,
    STEAL_MIN_REMAINING, TASK_FILE_VERSION,
};
use pico_core::core::history::TabHistory;
use std::fs;
use std::path::{Path, PathBuf};

fn seg(start: u64, end: u64, done: u64) -> Segment {
    Segment { start, end, done }
}

fn task(total: Option<u64>, segments: Vec<Segment>) -> TaskState {
    TaskState {
        version: TASK_FILE_VERSION,
        id: 7,
        url: "https://files.example.com/movie.mkv".to_string(),
        file_name: "movie.mkv".to_string(),
        total,
        etag: None,
        last_modified: None,
        connections: DEFAULT_CONNECTIONS,
        status: TaskStatus::Paused,
        added_ms: 1_000,
        segments,
        cookies: Vec::new(),
        error: None,
        referer: None,
        sha256: None,
        auto_retries: 0,
        error_hint: None,
    }
}

fn tmp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pico-v32-dl-test-{}-{tag}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("temp dir");
    dir
}

// ---------------------------------------------------------------------------
// plan_segments
// ---------------------------------------------------------------------------

#[test]
fn plan_zero_total_is_one_empty_segment() {
    assert_eq!(plan_segments(0, 8), vec![seg(0, 0, 0)]);
}

#[test]
fn plan_smaller_than_connections_gives_one_byte_segments() {
    // n = min(connections, total): no empty segments.
    assert_eq!(
        plan_segments(3, 8),
        vec![seg(0, 1, 0), seg(1, 2, 0), seg(2, 3, 0)]
    );
}

#[test]
fn plan_indivisible_size_gives_first_segments_the_extra_byte() {
    // 10 bytes over 3 connections: 4 + 3 + 3.
    assert_eq!(
        plan_segments(10, 3),
        vec![seg(0, 4, 0), seg(4, 7, 0), seg(7, 10, 0)]
    );
}

#[test]
fn plan_exact_split_is_even_and_contiguous() {
    let plan = plan_segments(1_000, 4);
    assert_eq!(
        plan,
        vec![
            seg(0, 250, 0),
            seg(250, 500, 0),
            seg(500, 750, 0),
            seg(750, 1_000, 0)
        ]
    );
}

#[test]
fn plan_connections_are_clamped() {
    assert_eq!(plan_segments(100, 0).len(), 1, "0 clamps up to 1");
    assert_eq!(
        plan_segments(10_000, 999).len(),
        MAX_CONNECTIONS as usize,
        "huge counts clamp down to MAX_CONNECTIONS"
    );
    assert_eq!(plan_segments(100, 1), vec![seg(0, 100, 0)]);
}

// ---------------------------------------------------------------------------
// Planner: claim / steal / release / progress
// ---------------------------------------------------------------------------

#[test]
fn claim_hands_out_fresh_segments_lowest_index_first() {
    let mut p = Planner::new(plan_segments(300, 3));
    assert_eq!(p.claim(), Some(Claim::Fresh(0)));
    assert_eq!(p.claim(), Some(Claim::Fresh(100)));
    assert_eq!(p.claim(), Some(Claim::Fresh(200)));
    // All assigned, each remaining (100) is below the steal
    // threshold: nothing more to give.
    assert_eq!(p.claim(), None);
}

#[test]
fn claim_never_steals_below_the_threshold() {
    let mut p = Planner::new(vec![seg(0, STEAL_MIN_REMAINING, 0)]);
    assert_eq!(p.claim(), Some(Claim::Fresh(0)));
    // remaining == STEAL_MIN_REMAINING is NOT > the threshold.
    assert_eq!(p.claim(), None);
}

#[test]
fn claim_steals_by_splitting_the_largest_remaining_segment() {
    let big = STEAL_MIN_REMAINING * 4;
    let mut p = Planner::new(vec![seg(0, big, 0), seg(big, big + 10, 0)]);
    assert_eq!(p.claim(), Some(Claim::Fresh(0)));
    assert_eq!(p.claim(), Some(Claim::Fresh(big)));
    // No fresh work: steal from segment 0 (the largest
    // remaining). Position 0 + remaining/2 = big/2.
    match p.claim() {
        Some(Claim::Steal { from, start }) => {
            assert_eq!(from, 0);
            assert_eq!(p.segment(0), Some(seg(0, big / 2, 0)));
            assert_eq!(p.segment(start), Some(seg(big / 2, big, 0)));
        }
        other => panic!("expected a steal, got {other:?}"),
    }
}

#[test]
fn steal_split_accounts_for_progress_already_made() {
    let total = STEAL_MIN_REMAINING * 4;
    let mut s = seg(0, total, 0);
    s.done = STEAL_MIN_REMAINING; // position = 256 KiB, remaining = 768 KiB
    let mut p = Planner::new(vec![s]);
    assert_eq!(p.claim(), Some(Claim::Fresh(0)));
    match p.claim() {
        Some(Claim::Steal { from, start }) => {
            assert_eq!(from, 0);
            // split = position + remaining/2 = 256K + 384K.
            let split = STEAL_MIN_REMAINING + (STEAL_MIN_REMAINING * 3) / 2;
            assert_eq!(p.segment(0), Some(seg(0, split, STEAL_MIN_REMAINING)));
            assert_eq!(p.segment(start), Some(seg(split, total, 0)));
        }
        other => panic!("expected a steal, got {other:?}"),
    }
}

#[test]
fn finished_segments_are_never_stolen_or_claimed() {
    let big = STEAL_MIN_REMAINING * 4;
    let mut p = Planner::new(vec![seg(0, big, big), seg(big, big + big, 0)]);
    // Segment 0 is finished; only segment 1 is claimable.
    assert_eq!(p.claim(), Some(Claim::Fresh(big)));
    // Segment 1 can be stolen from; the finished segment 0
    // must never be the steal source even though it is big.
    match p.claim() {
        Some(Claim::Steal { from, .. }) => assert_eq!(from, big),
        other => panic!("expected a steal from segment 1, got {other:?}"),
    }
}

#[test]
fn steal_only_happens_when_no_fresh_work_remains() {
    let big = STEAL_MIN_REMAINING * 4;
    let mut p = Planner::new(vec![seg(0, big, 0), seg(big, big + 50, 0)]);
    assert_eq!(p.claim(), Some(Claim::Fresh(0)));
    // Segment 0 is stealable, but segment 1 is still fresh:
    // fresh work wins over stealing.
    assert_eq!(p.claim(), Some(Claim::Fresh(big)));
}

#[test]
fn release_unassigns_and_progress_is_monotonic_and_clamped() {
    let mut p = Planner::new(plan_segments(100, 1));
    assert_eq!(p.claim(), Some(Claim::Fresh(0)));
    p.progress(0, 40);
    p.progress(0, 10); // backwards: ignored
    assert_eq!(p.downloaded(), 40);
    p.progress(0, 999); // past the end: clamped to len
    assert_eq!(p.downloaded(), 100);
    assert!(p.all_finished());

    let mut p = Planner::new(plan_segments(100, 2));
    assert_eq!(p.claim(), Some(Claim::Fresh(0)));
    p.release(0);
    assert_eq!(
        p.claim(),
        Some(Claim::Fresh(0)),
        "released work is claimable again"
    );
    p.release(55); // no segment starts there: no panic, no effect
}

// ---------------------------------------------------------------------------
// Planner::replan_pending
// ---------------------------------------------------------------------------

#[test]
fn replan_resplits_only_pending_ranges() {
    // 1000 bytes in 2 segments; segment 0 is in flight with
    // progress, segment 1 is pending. Asking for 4 connections
    // must leave segment 0 untouched and re-split [500,1000)
    // into 4 - 1 (started) = 3 fresh segments.
    let mut p = Planner::new(plan_segments(1_000, 2));
    assert_eq!(p.claim(), Some(Claim::Fresh(0)));
    p.progress(0, 100);
    p.replan_pending(4);
    let segs = p.segments().to_vec();
    assert_eq!(segs[0], seg(0, 500, 100), "in-flight segment untouched");
    let pending: Vec<Segment> = segs[1..].to_vec();
    assert_eq!(pending.len(), 3, "pending re-split into 3: {pending:?}");
    // The pending pieces cover exactly [500, 1000), disjoint
    // and gapless.
    let mut cursor = 500;
    for s in &pending {
        assert_eq!(s.start, cursor);
        assert_eq!(s.done, 0);
        cursor = s.end;
    }
    assert_eq!(cursor, 1_000);
    assert_eq!(p.downloaded(), 100, "progress survives the replan");
}

#[test]
fn replan_keeps_finished_and_partial_unassigned_segments() {
    // Segment 0 finished, segment 1 partially downloaded but
    // currently unassigned (released), segment 2 pending.
    let mut p = Planner::new(vec![seg(0, 100, 100), seg(100, 200, 30), seg(200, 400, 0)]);
    p.replan_pending(2);
    let segs = p.segments().to_vec();
    assert!(segs.contains(&seg(0, 100, 100)), "finished kept: {segs:?}");
    assert!(segs.contains(&seg(100, 200, 30)), "partial kept: {segs:?}");
    // Kept segments that still need a connection: just the
    // partial one (the finished segment is inert), so
    // k = max(2-1, 1) = 1 fresh segment covering the pending
    // range.
    assert!(
        segs.contains(&seg(200, 400, 0)),
        "pending re-planned: {segs:?}"
    );
    assert_eq!(segs.len(), 3);
}

#[test]
fn replan_unknown_total_is_a_noop() {
    let open = vec![seg(0, 0, 1234)];
    let mut p = Planner::new(open.clone());
    p.replan_pending(16);
    assert_eq!(p.segments(), open.as_slice());
}

#[test]
fn replan_with_no_pending_bytes_changes_nothing() {
    let mut p = Planner::new(vec![seg(0, 100, 100)]);
    p.replan_pending(8);
    assert_eq!(p.segments(), &[seg(0, 100, 100)]);
}

#[test]
fn planner_sync_into_writes_segments_back_to_the_task() {
    let mut t = task(Some(300), plan_segments(300, 3));
    let mut p = Planner::from_task(&t);
    assert_eq!(p.claim(), Some(Claim::Fresh(0)));
    p.progress(0, 42);
    p.sync_into(&mut t);
    assert_eq!(t.segments[0].done, 42);
    assert_eq!(t.downloaded(), 42);
}

// ---------------------------------------------------------------------------
// sanitize_file_name
// ---------------------------------------------------------------------------

#[test]
fn sanitize_strips_path_traversal_and_separators() {
    assert_eq!(sanitize_file_name("../../etc/passwd"), "passwd");
    assert_eq!(sanitize_file_name("..\\..\\evil.exe"), "evil.exe");
    assert_eq!(sanitize_file_name("a/b\\c.txt"), "c.txt");
    assert_eq!(sanitize_file_name(".."), "download");
    assert_eq!(sanitize_file_name(""), "download");
}

#[test]
fn sanitize_strips_bidi_and_zero_width_format_chars() {
    // AUDIT 0.40.1: category-Cf characters (bidi controls,
    // zero-width marks, BOM) survive `is_control` filtering
    // but can render a file name's extension reversed in
    // Explorer (the U+202E spoof) — they are stripped.
    assert_eq!(
        sanitize_file_name("invoice\u{202e}fdp.exe"),
        "invoicefdp.exe"
    );
    assert_eq!(sanitize_file_name("setup\u{200b}\u{feff}.exe"), "setup.exe");
    assert_eq!(
        sanitize_file_name("\u{202a}report\u{202c}.pdf"),
        "report.pdf"
    );
    // Ordinary non-ASCII letters are NOT format chars.
    assert_eq!(sanitize_file_name("گزارش.pdf"), "گزارش.pdf");
}

#[test]
fn sanitize_removes_forbidden_and_control_characters() {
    assert_eq!(sanitize_file_name("a<b>c:d\"e|f?g*h.txt"), "abcdefgh.txt");
    assert_eq!(sanitize_file_name("na\u{0007}me\u{001f}.txt"), "name.txt");
}

#[test]
fn sanitize_trims_trailing_dots_and_spaces_and_leading_spaces() {
    assert_eq!(sanitize_file_name("file.txt   "), "file.txt");
    assert_eq!(sanitize_file_name("file..."), "file");
    assert_eq!(sanitize_file_name("   file.txt"), "file.txt");
    assert_eq!(sanitize_file_name("file. . ."), "file");
}

#[test]
fn sanitize_prefixes_windows_reserved_names() {
    assert_eq!(sanitize_file_name("CON.txt"), "_CON.txt");
    assert_eq!(sanitize_file_name("con"), "_con");
    assert_eq!(sanitize_file_name("NUL"), "_NUL");
    assert_eq!(sanitize_file_name("com1.log"), "_com1.log");
    assert_eq!(sanitize_file_name("Lpt9"), "_Lpt9");
    assert_eq!(sanitize_file_name("console.txt"), "console.txt");
    assert_eq!(sanitize_file_name("COM10.txt"), "COM10.txt");
}

#[test]
fn sanitize_caps_length_preserving_the_extension() {
    let long = format!("{}.mkv", "a".repeat(300));
    let out = sanitize_file_name(&long);
    assert_eq!(out.chars().count(), 200);
    assert!(out.ends_with(".mkv"), "extension preserved: {out}");
    let no_ext = "b".repeat(250);
    assert_eq!(sanitize_file_name(&no_ext).chars().count(), 200);
    // A suffix longer than 10 chars is not an extension: the
    // whole name is truncated flat.
    let long_suffix = format!("{}.abcdefghijklm", "c".repeat(300));
    let out = sanitize_file_name(&long_suffix);
    assert_eq!(out.chars().count(), 200);
    assert!(!out.ends_with(".abcdefghijklm"));
}

// ---------------------------------------------------------------------------
// file_name_from_url
// ---------------------------------------------------------------------------

#[test]
fn url_file_name_takes_the_last_path_segment() {
    assert_eq!(
        file_name_from_url("https://cdn.example.com/a/b/movie.mkv?token=x#frag"),
        "movie.mkv"
    );
    assert_eq!(file_name_from_url("https://example.com/dir/"), "dir");
    assert_eq!(file_name_from_url("https://example.com/"), "download");
    assert_eq!(file_name_from_url("https://example.com"), "download");
    assert_eq!(
        file_name_from_url("https://example.com/?file=x.zip"),
        "download"
    );
}

#[test]
fn url_file_name_percent_decodes_exactly_once() {
    assert_eq!(
        file_name_from_url("https://example.com/my%20file%20%28final%29.zip"),
        "my file (final).zip"
    );
    // %2520 decodes once to the literal text "%20", not a space.
    assert_eq!(
        file_name_from_url("https://example.com/a%2520b.txt"),
        "a%20b.txt"
    );
    // Invalid escapes stay literal.
    assert_eq!(
        file_name_from_url("https://example.com/a%zzb.txt"),
        "a%zzb.txt"
    );
    assert_eq!(file_name_from_url("https://example.com/a%2.txt"), "a%2.txt");
    // A decoded separator cannot smuggle a path through.
    assert_eq!(file_name_from_url("https://example.com/x%2Fy.txt"), "y.txt");
}

// ---------------------------------------------------------------------------
// file_name_from_disposition
// ---------------------------------------------------------------------------

#[test]
fn disposition_prefers_the_rfc5987_star_form() {
    let h = "attachment; filename=\"plain.txt\"; filename*=UTF-8''fancy%20name.txt";
    assert_eq!(
        file_name_from_disposition(h).as_deref(),
        Some("fancy name.txt")
    );
    let star_only = "attachment; filename*=UTF-8''caf%C3%A9.pdf";
    assert_eq!(
        file_name_from_disposition(star_only).as_deref(),
        Some("café.pdf")
    );
}

#[test]
fn disposition_quoted_and_token_forms() {
    assert_eq!(
        file_name_from_disposition("attachment; filename=\"report 2026.pdf\"").as_deref(),
        Some("report 2026.pdf")
    );
    assert_eq!(
        file_name_from_disposition("attachment; filename=data.bin").as_deref(),
        Some("data.bin")
    );
    // The raw name comes back unsanitized — sanitizing is the
    // caller's step (the brief's contract).
    assert_eq!(
        file_name_from_disposition("attachment; filename=\"../evil.sh\"").as_deref(),
        Some("../evil.sh")
    );
}

#[test]
fn disposition_absent_or_empty_is_none() {
    assert_eq!(file_name_from_disposition("attachment"), None);
    assert_eq!(
        file_name_from_disposition("attachment; filename=\"\""),
        None
    );
    assert_eq!(file_name_from_disposition(""), None);
}

// ---------------------------------------------------------------------------
// dedupe_file_name
// ---------------------------------------------------------------------------

#[test]
fn dedupe_returns_the_name_when_free_and_sequences_otherwise() {
    assert_eq!(dedupe_file_name("a.txt", |_| false), "a.txt");
    let taken = |n: &str| n == "a.txt";
    assert_eq!(dedupe_file_name("a.txt", taken), "a (1).txt");
    let taken2 = |n: &str| ["a.txt", "a (1).txt"].contains(&n);
    assert_eq!(dedupe_file_name("a.txt", taken2), "a (2).txt");
    // No extension: the counter rides the whole name.
    let taken3 = |n: &str| n == "README";
    assert_eq!(dedupe_file_name("README", taken3), "README (1)");
}

#[test]
fn dedupe_falls_back_to_the_last_tried_at_the_cap() {
    // The counter runs to 99_999 before giving up: a name a
    // caller can never collide with is worth a longer search
    // than 999 (an exhausted fallback would be handed to a
    // rename that REPLACES the existing file).
    assert_eq!(dedupe_file_name("a.txt", |_| true), "a (99999).txt");
}

// ---------------------------------------------------------------------------
// Cookies / hosts / URL gates
// ---------------------------------------------------------------------------

fn cookie(name: &str, domain: &str, host_only: bool, secure: bool) -> CookiePair {
    CookiePair {
        name: name.to_string(),
        value: "v".to_string(),
        domain: domain.to_string(),
        host_only,
        secure,
        path: "/".to_string(),
        expires_unix_ms: None,
    }
}

#[test]
fn cookie_host_only_matches_the_exact_host() {
    let cs = vec![cookie("a", "example.com", true, false)];
    assert_eq!(
        cookie_header(&cs, "https://example.com/f").as_deref(),
        Some("a=v")
    );
    assert_eq!(cookie_header(&cs, "https://sub.example.com/f"), None);
}

#[test]
fn cookie_domain_cookie_covers_subdomains_but_not_lookalikes() {
    let cs = vec![cookie("a", "example.com", false, false)];
    assert_eq!(
        cookie_header(&cs, "https://example.com/f").as_deref(),
        Some("a=v")
    );
    assert_eq!(
        cookie_header(&cs, "https://deep.sub.example.com/f").as_deref(),
        Some("a=v")
    );
    assert_eq!(cookie_header(&cs, "https://notexample.com/f"), None);
    assert_eq!(cookie_header(&cs, "https://example.com.evil.net/f"), None);
}

#[test]
fn cookie_secure_is_https_only_and_foreign_domains_never_match() {
    let cs = vec![
        cookie("s", "example.com", false, true),
        cookie("x", "other.net", false, false),
    ];
    assert_eq!(cookie_header(&cs, "http://example.com/f"), None);
    assert_eq!(
        cookie_header(&cs, "https://example.com/f").as_deref(),
        Some("s=v"),
        "the foreign-domain cookie stays out even on https"
    );
}

#[test]
fn cookie_header_joins_pairs_and_is_none_when_empty() {
    let cs = vec![
        cookie("a", "example.com", false, false),
        cookie("b", "example.com", false, false),
    ];
    assert_eq!(
        cookie_header(&cs, "https://example.com/").as_deref(),
        Some("a=v; b=v")
    );
    assert_eq!(cookie_header(&[], "https://example.com/"), None);
}

#[test]
fn url_host_and_downloadable_gate() {
    assert_eq!(
        url_host("https://User@EXAMPLE.com:8443/a?b").as_deref(),
        Some("example.com")
    );
    assert_eq!(
        url_host("http://127.0.0.1:5173/").as_deref(),
        Some("127.0.0.1")
    );
    assert_eq!(
        url_host("https://[2001:db8::1]:443/x").as_deref(),
        Some("2001:db8::1")
    );
    assert_eq!(url_host("not a url"), None);
    assert!(is_downloadable_url("https://example.com/f.zip"));
    assert!(is_downloadable_url("HTTP://example.com/f.zip"));
    assert!(!is_downloadable_url("ftp://example.com/f.zip"));
    assert!(!is_downloadable_url("file:///C:/f.zip"));
    assert!(!is_downloadable_url("https:///no-host"));
}

// ---------------------------------------------------------------------------
// resume_decision
// ---------------------------------------------------------------------------

fn probe(total: Option<u64>, accept_ranges: bool) -> ProbeFacts {
    ProbeFacts {
        status: 200,
        total,
        accept_ranges,
        etag: None,
        last_modified: None,
    }
}

fn downloaded_task() -> TaskState {
    let mut t = task(Some(1_000), plan_segments(1_000, 2));
    t.segments[0].done = 250;
    t
}

#[test]
fn resume_fresh_when_nothing_is_downloaded() {
    let t = task(Some(1_000), plan_segments(1_000, 2));
    assert_eq!(
        resume_decision(&t, &probe(Some(1_000), true)),
        ResumeDecision::Fresh
    );
    // Even a hostile probe cannot make a zero-progress task
    // "restart" — there is nothing to lose.
    assert_eq!(
        resume_decision(&t, &probe(Some(9), false)),
        ResumeDecision::Fresh
    );
}

#[test]
fn resume_happy_path_and_validator_matrix() {
    let mut t = downloaded_task();
    assert_eq!(
        resume_decision(&t, &probe(Some(1_000), true)),
        ResumeDecision::Resume
    );
    // Matching validators keep the resume.
    t.etag = Some("\"abc\"".to_string());
    let mut p = probe(Some(1_000), true);
    p.etag = Some("\"abc\"".to_string());
    assert_eq!(resume_decision(&t, &p), ResumeDecision::Resume);
    // A validator present on one side only falls back to the
    // remaining checks (total equality is the floor).
    p.etag = None;
    assert_eq!(resume_decision(&t, &p), ResumeDecision::Resume);
    // Differing validators on both sides restart.
    p.etag = Some("\"different\"".to_string());
    assert_eq!(resume_decision(&t, &p), ResumeDecision::Restart);
    // Same for Last-Modified.
    let mut t2 = downloaded_task();
    t2.last_modified = Some("Mon, 01 Jan 2026 00:00:00 GMT".to_string());
    let mut p2 = probe(Some(1_000), true);
    p2.last_modified = Some("Tue, 02 Jan 2026 00:00:00 GMT".to_string());
    assert_eq!(resume_decision(&t2, &p2), ResumeDecision::Restart);
}

#[test]
fn resume_restarts_on_total_mismatch_missing_total_or_lost_ranges() {
    let t = downloaded_task();
    assert_eq!(
        resume_decision(&t, &probe(Some(2_000), true)),
        ResumeDecision::Restart
    );
    assert_eq!(
        resume_decision(&t, &probe(None, true)),
        ResumeDecision::Restart
    );
    assert_eq!(
        resume_decision(&t, &probe(Some(1_000), false)),
        ResumeDecision::Restart
    );
}

#[test]
fn resume_unknown_total_with_progress_always_restarts() {
    // The open-ended marker segment: downloaded > 0 with no
    // known total can never resume, whatever the probe says.
    let t = task(None, vec![seg(0, 0, 500)]);
    assert_eq!(
        resume_decision(&t, &probe(Some(1_000), true)),
        ResumeDecision::Restart
    );
    assert_eq!(
        resume_decision(&t, &probe(None, true)),
        ResumeDecision::Restart
    );
}

// ---------------------------------------------------------------------------
// parse_content_range
// ---------------------------------------------------------------------------

#[test]
fn content_range_parses_start_end_total() {
    assert_eq!(
        parse_content_range("bytes 0-1023/2048"),
        Some((0, 1023, 2048))
    );
    assert_eq!(
        parse_content_range("bytes 500-999/1000"),
        Some((500, 999, 1000))
    );
    assert_eq!(parse_content_range("bytes 0-0/1"), Some((0, 0, 1)));
}

#[test]
fn content_range_rejects_garbage_and_unknown_totals() {
    assert_eq!(parse_content_range("bytes 0-1023/*"), None);
    assert_eq!(parse_content_range("bytes */2048"), None);
    assert_eq!(parse_content_range("items 0-1/2"), None);
    assert_eq!(
        parse_content_range("bytes 100-50/2048"),
        None,
        "start past end"
    );
    assert_eq!(
        parse_content_range("bytes 0-2048/2048"),
        None,
        "end past total"
    );
    assert_eq!(parse_content_range(""), None);
}

// ---------------------------------------------------------------------------
// TaskState: JSON round-trip + sanitized()
// ---------------------------------------------------------------------------

#[test]
fn task_state_json_round_trips() {
    let mut t = task(Some(1_000), plan_segments(1_000, 4));
    t.etag = Some("\"v1\"".to_string());
    t.status = TaskStatus::Downloading;
    t.cookies = vec![cookie("sid", "example.com", false, true)];
    let text = serde_json::to_string(&t).expect("serialize");
    let back: TaskState = serde_json::from_str(&text).expect("deserialize");
    assert_eq!(back, t);
    assert_eq!(TaskStatus::Complete.label(), "done");
    assert_eq!(TaskStatus::Queued.label(), "queued");
}

#[test]
fn sanitized_rebuilds_a_corrupt_segment_map_from_the_total() {
    // Overlapping segments: the progress map cannot be
    // trusted, so sanitization rebuilds a fresh zero-progress
    // plan from the total instead of splicing wrong bytes.
    let mut t = task(Some(1_000), vec![seg(0, 600, 100), seg(500, 1_000, 100)]);
    t.connections = 2;
    let clean = t.sanitized().expect("recoverable");
    assert_eq!(clean.segments, plan_segments(1_000, 2));
    assert_eq!(clean.downloaded(), 0);

    // done past the segment end: same rebuild.
    let t = task(Some(100), vec![seg(0, 100, 500)]);
    let clean = t.sanitized().expect("recoverable");
    assert_eq!(clean.downloaded(), 0);
    assert_eq!(clean.segments, plan_segments(100, DEFAULT_CONNECTIONS));

    // A segment reaching past the total: same rebuild.
    let t = task(Some(100), vec![seg(0, 200, 0)]);
    assert_eq!(t.sanitized().expect("recoverable").downloaded(), 0);
}

#[test]
fn sanitized_keeps_an_honest_segment_map_and_clamps_fields() {
    let mut t = task(Some(1_000), plan_segments(1_000, 2));
    t.segments[0].done = 500;
    t.connections = 0; // → DEFAULT
    t.file_name = "../CON.txt".to_string();
    t.error = Some("x".repeat(500));
    let clean = t.sanitized().expect("recoverable");
    assert_eq!(clean.downloaded(), 500, "honest progress survives");
    assert_eq!(clean.connections, DEFAULT_CONNECTIONS);
    assert_eq!(clean.file_name, "_CON.txt");
    assert_eq!(clean.error.as_ref().map(|e| e.chars().count()), Some(300));

    let mut t = task(Some(10), plan_segments(10, 1));
    t.connections = 999;
    assert_eq!(t.sanitized().expect("ok").connections, MAX_CONNECTIONS);
}

#[test]
fn sanitized_repairs_a_complete_records_missing_total() {
    // AUDIT 0.34.2 (G3): live completion forces a done
    // task's total to the file size, but a legacy record
    // can persist `total: null` under Complete — the
    // snapshot then carried percent: null for a finished
    // file and the renderers drew an unfinished bar.
    // Sanitization repairs the total from the segment
    // map's progress sum (the unknown-total map is the
    // single open-ended segment, whose `done` IS the
    // finished size here).
    let mut t = task(None, vec![seg(0, 0, 12_345)]);
    t.status = TaskStatus::Complete;
    let clean = t.sanitized().expect("recoverable");
    assert_eq!(clean.total, Some(12_345));
    assert_eq!(clean.downloaded(), 12_345);

    // A non-Complete task keeps its unknown total: the
    // repair is only for finished files.
    let t = task(None, vec![seg(0, 0, 500)]);
    let clean = t.sanitized().expect("recoverable");
    assert_eq!(clean.total, None);
}

#[test]
fn sanitized_zeroes_an_out_of_range_added_ms() {
    // AUDIT 0.34.2 (G5): `added_ms` feeds a JS Date on
    // the downloads page; a crafted stamp outside the JS
    // Date range rendered as the literal text
    // "Invalid Date". Past the year-9999 stamp the value
    // is not a date — zero it (unknown). A real stamp is
    // untouched.
    let mut t = task(Some(10), plan_segments(10, 1));
    t.added_ms = u64::MAX;
    assert_eq!(t.sanitized().expect("ok").added_ms, 0);
    let mut t = task(Some(10), plan_segments(10, 1));
    t.added_ms = 1_700_000_000_000;
    assert_eq!(t.sanitized().expect("ok").added_ms, 1_700_000_000_000);
}

#[test]
fn sanitized_rejects_undownloadable_urls() {
    let mut t = task(Some(10), plan_segments(10, 1));
    t.url = "file:///etc/passwd".to_string();
    assert!(t.sanitized().is_none());
    let mut t = task(Some(10), plan_segments(10, 1));
    t.url = "javascript:alert(1)".to_string();
    assert!(t.sanitized().is_none());
}

#[test]
fn sanitized_caps_cookies_and_strips_control_characters() {
    let mut t = task(Some(10), plan_segments(10, 1));
    let mut c = cookie("si\u{0001}d", "EXAMPLE.com", false, false);
    c.value = "v".repeat(9000);
    t.cookies = vec![c];
    for i in 0..100 {
        t.cookies
            .push(cookie(&format!("c{i}"), "example.com", false, false));
    }
    let clean = t.sanitized().expect("ok");
    assert_eq!(clean.cookies.len(), 64, "cookie count capped");
    assert_eq!(clean.cookies[0].name, "sid", "control chars stripped");
    assert_eq!(
        clean.cookies[0].value.len(),
        8192,
        "value capped at 8192 bytes"
    );
    // A cookie whose name is ALL control chars is dropped.
    let mut t = task(Some(10), plan_segments(10, 1));
    t.cookies = vec![cookie("\u{0002}\u{0003}", "example.com", false, false)];
    assert!(t.sanitized().expect("ok").cookies.is_empty());
}

// ---------------------------------------------------------------------------
// Persisted task files: save / load / delete
// ---------------------------------------------------------------------------

#[test]
fn save_load_round_trip_through_a_temp_dir() {
    let dir = tmp("roundtrip");
    let mut a = task(Some(2_000), plan_segments(2_000, 2));
    a.id = 11;
    a.added_ms = 500;
    a.status = TaskStatus::Downloading; // on disk mid-flight
    let mut b = task(Some(10), plan_segments(10, 1));
    b.id = 12;
    b.added_ms = 100; // older: sorts first
    save_task(&dir, &a).expect("save a");
    save_task(&dir, &b).expect("save b");
    assert!(dir.join("11.json").is_file());
    // No temp residue is left behind by the atomic write.
    let names: Vec<String> = fs::read_dir(&dir)
        .expect("read dir")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        names.iter().all(|n| !n.contains(".tmp")),
        "residue: {names:?}"
    );

    let loaded = load_tasks(&dir);
    assert_eq!(loaded.len(), 2);
    assert_eq!(loaded[0].id, 12, "sorted by added_ms");
    assert_eq!(loaded[1].id, 11);
    assert_eq!(
        loaded[1].status,
        TaskStatus::Paused,
        "a task that was Downloading on disk loads as Paused"
    );

    delete_task_file(&dir, 11);
    let loaded = load_tasks(&dir);
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].id, 12);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn load_tasks_restores_every_stored_status() {
    // 0.35 (WS3): the full restore table through
    // save_task → load_tasks. Downloading→Paused and
    // Queued→Paused are pinned by the round-trip and
    // dedupe tests above; this completes the table for
    // the three statuses that restore as themselves.
    let dir = tmp("restore-table");
    let cases = [
        (31, TaskStatus::Paused, TaskStatus::Paused),
        (32, TaskStatus::Failed, TaskStatus::Failed),
        (33, TaskStatus::Complete, TaskStatus::Complete),
        (34, TaskStatus::Downloading, TaskStatus::Paused),
        (35, TaskStatus::Queued, TaskStatus::Paused),
    ];
    for (id, stored, _) in cases {
        let mut t = task(Some(10), plan_segments(10, 1));
        t.id = id;
        t.file_name = format!("restore-{id}.bin");
        t.added_ms = id * 10;
        t.status = stored;
        if stored == TaskStatus::Failed {
            t.error = Some("HTTP 404".to_string());
        }
        save_task(&dir, &t).expect("save task");
    }
    let loaded = load_tasks(&dir);
    assert_eq!(loaded.len(), cases.len());
    for (id, stored, expected) in cases {
        let found = loaded
            .iter()
            .find(|t| t.id == id)
            .unwrap_or_else(|| panic!("task {id} (stored {stored:?}) missing"));
        assert_eq!(
            found.status, expected,
            "stored {stored:?} must restore as {expected:?}"
        );
    }
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn load_tasks_skips_corrupt_and_oversized_files() {
    let dir = tmp("corrupt");
    fs::write(dir.join("1.json"), "{not json").expect("write corrupt");
    // Unknown status string: serde fails, the file is skipped.
    fs::write(
        dir.join("2.json"),
        "{\"version\":1,\"id\":2,\"url\":\"https://e.com/f\",\"file_name\":\"f\",\"total\":1,\"etag\":null,\"last_modified\":null,\"connections\":8,\"status\":\"bogus\",\"added_ms\":1,\"segments\":[],\"cookies\":[],\"error\":null}",
    )
    .expect("write bad status");
    // Over the 1 MiB cap: skipped without being parsed.
    fs::write(dir.join("3.json"), " ".repeat(1_048_577)).expect("write oversized");
    let good = task(Some(10), plan_segments(10, 1));
    save_task(&dir, &good).expect("save good");
    let loaded = load_tasks(&dir);
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].id, 7);
    // A missing directory loads as an empty list, not an error.
    assert!(load_tasks(&dir.join("nope")).is_empty());
    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// SpeedMeter + format helpers
// ---------------------------------------------------------------------------

#[test]
fn speed_meter_rates_the_trailing_window() {
    let mut m = SpeedMeter::new();
    assert_eq!(m.rate(0), 0.0, "no samples");
    m.record(0, 0);
    assert_eq!(m.rate(0), 0.0, "one sample");
    m.record(1_000, 5_000);
    assert_eq!(m.rate(1_000), 5_000.0);
    // Samples older than 5 s from the newest fall out: at
    // t=6000 only the t>=1000 samples count... the t=0 sample
    // is dropped by the record at t=6000.
    m.record(6_000, 65_000);
    // Window now holds (1000, 5000) and (6000, 65000):
    // 60000 bytes over 5 s.
    assert_eq!(m.rate(6_000), 12_000.0);
}

#[test]
fn speed_meter_short_intervals_and_backwards_bytes_give_zero() {
    let mut m = SpeedMeter::new();
    m.record(1_000, 0);
    m.record(1_100, 999_999); // dt 100 ms < 250 ms floor
    assert_eq!(m.rate(1_100), 0.0);
    // Bytes going backwards reset the baseline.
    let mut m = SpeedMeter::new();
    m.record(0, 10_000);
    m.record(1_000, 5_000);
    assert_eq!(
        m.rate(1_000),
        0.0,
        "a backwards step is not a negative rate"
    );
    m.record(2_000, 15_000);
    assert_eq!(m.rate(2_000), 10_000.0, "the new baseline rates cleanly");
    m.reset();
    assert_eq!(m.rate(2_000), 0.0);
}

#[test]
fn eta_percent_and_text_helpers() {
    assert_eq!(eta_seconds(Some(10_000), 5_000, 1_000.0), Some(5));
    assert_eq!(eta_seconds(Some(10_000), 10_000, 1_000.0), Some(0));
    assert_eq!(eta_seconds(None, 5_000, 1_000.0), None);
    assert_eq!(eta_seconds(Some(10_000), 5_000, 0.0), None);
    assert_eq!(percent(250, Some(1_000)), Some(25.0));
    assert_eq!(percent(2_000, Some(1_000)), Some(100.0), "clamped");
    assert_eq!(percent(10, None), None);
    assert_eq!(speed_text(1_500.0), "1.5 KB/s");
    assert_eq!(speed_text(0.0), "0 B/s");
    assert_eq!(eta_text(None), "—");
    assert_eq!(eta_text(Some(90)), "1.5 min");
    assert_eq!(eta_text(Some(45)), "45 s");
}

// ---------------------------------------------------------------------------
// resolve_downloads_dir
// ---------------------------------------------------------------------------

#[test]
fn resolve_downloads_dir_prefers_the_exe_folder_when_writable() {
    let exe = Path::new("D:/Pico");
    let user = Path::new("C:/Users/x/Downloads");
    assert_eq!(
        resolve_downloads_dir(exe, Some(user), true),
        exe.join("Downloads")
    );
    assert_eq!(
        resolve_downloads_dir(exe, Some(user), false),
        user.to_path_buf(),
        "not writable: the user's Downloads folder"
    );
    assert_eq!(
        resolve_downloads_dir(exe, None, false),
        exe.join("Downloads"),
        "no user folder known: exe dir is the last resort"
    );
}

// ---------------------------------------------------------------------------
// 0.32.1 audit regressions
// ---------------------------------------------------------------------------

#[test]
fn open_ended_planner_claims_the_marker_exactly_once() {
    // AUDIT 0.32.1 (P4-A1): an unknown-total task's marker
    // segment {0,0,0} has remaining() == 0, so a planner that
    // only hands out unfinished segments NEVER claims it and
    // the "download" completes instantly as a 0-byte file.
    // An open-ended planner hands the marker out once; the
    // stream's EOF — never the planner — ends it.
    let mut p = Planner::new_open_ended(vec![seg(0, 0, 0)]);
    assert_eq!(p.claim(), Some(Claim::Fresh(0)));
    assert_eq!(p.claim(), None, "the marker is claimed once only");
    p.progress(0, 500);
    assert_eq!(p.downloaded(), 500, "streamed bytes accrue on the marker");
    // A CLOSED planner over an empty segment (a known
    // zero-byte total) must claim nothing: there is no stream
    // to fetch.
    let mut closed = Planner::new(vec![seg(0, 0, 0)]);
    assert_eq!(closed.claim(), None);
}

#[test]
fn steal_inserts_in_sorted_order_and_the_map_stays_honest() {
    // AUDIT 0.32.1 (P4-A2): stealing used to PUSH the split
    // segment at the end of the vector, leaving the persisted
    // map unsorted — load_tasks then declared it dishonest
    // and discarded ALL progress on the next launch.
    let s = STEAL_MIN_REMAINING;
    let total = 4 * s + 200;
    let mut p = Planner::new(vec![
        seg(0, 4 * s, 0),
        seg(4 * s, 4 * s + 100, 0),
        seg(4 * s + 100, total, 0),
    ]);
    assert_eq!(p.claim(), Some(Claim::Fresh(0)));
    assert_eq!(p.claim(), Some(Claim::Fresh(4 * s)));
    assert_eq!(p.claim(), Some(Claim::Fresh(4 * s + 100)));
    p.progress(4 * s, 50);
    // Steal from segment 0 (not the vector's last): the split
    // segment must land in sorted position, not at the end.
    match p.claim() {
        Some(Claim::Steal { from, start }) => {
            assert_eq!(from, 0);
            assert_eq!(start, 2 * s);
        }
        other => panic!("expected a steal, got {other:?}"),
    }
    let starts: Vec<u64> = p.segments().iter().map(|x| x.start).collect();
    let mut sorted = starts.clone();
    sorted.sort_unstable();
    assert_eq!(starts, sorted, "segments stay sorted by start: {starts:?}");
    // And the map survives the sanitizer verbatim — progress
    // included — instead of being rebuilt from zero.
    let mut t = task(Some(total), Vec::new());
    p.sync_into(&mut t);
    let expected = t.segments.clone();
    let clean = t.sanitized().expect("honest map");
    assert_eq!(clean.segments, expected);
    assert_eq!(clean.downloaded(), 50);
}

#[test]
fn replan_produces_a_sorted_honest_map() {
    // AUDIT 0.32.1 (P4-A2): replan_pending used to append the
    // fresh segments after ALL kept ones, so a kept partial
    // segment sitting between two pending ranges left the
    // persisted map unsorted (and dishonest on reload).
    let mut p = Planner::new(vec![
        seg(0, 100, 100),  // finished
        seg(100, 500, 0),  // pending
        seg(500, 600, 50), // partial, unassigned
        seg(600, 1000, 0), // pending
    ]);
    p.replan_pending(4);
    let segs = p.segments().to_vec();
    let mut cursor = 0u64;
    for s in &segs {
        assert_eq!(s.start, cursor, "sorted + gapless: {segs:?}");
        cursor = s.end;
    }
    assert_eq!(cursor, 1000);
    let mut t = task(Some(1000), Vec::new());
    p.sync_into(&mut t);
    let expected = t.segments.clone();
    let clean = t.sanitized().expect("honest map");
    assert_eq!(clean.segments, expected);
    assert_eq!(clean.downloaded(), 150, "progress survives the replan");
}

#[test]
fn replan_budget_counts_only_segments_that_need_a_connection() {
    // AUDIT 0.32.1 (P4-A7): the replan budget used to charge
    // FINISHED segments against the connection count, so the
    // pending range was split into fewer pieces than asked.
    let mut p = Planner::new(vec![
        seg(0, 100, 100),
        seg(100, 200, 100),
        seg(200, 1000, 0),
    ]);
    p.replan_pending(3);
    let pending: Vec<&Segment> = p.segments().iter().filter(|s| s.start >= 200).collect();
    assert_eq!(
        pending.len(),
        3,
        "both finished segments are inert: the pending range gets all 3 connections: {:?}",
        p.segments()
    );
}

#[test]
fn progress_by_start_key_survives_a_steal_insert() {
    // AUDIT 0.32.1 (P1-10/P4-A8): workers address segments by
    // their START, which no split, steal or replan can move —
    // an index into a rebuilt vector could silently name a
    // different byte range mid-flight.
    let s = STEAL_MIN_REMAINING;
    let mut p = Planner::new(vec![seg(0, 4 * s, 0), seg(4 * s, 4 * s + 100, 0)]);
    assert_eq!(p.claim(), Some(Claim::Fresh(0)));
    assert_eq!(p.claim(), Some(Claim::Fresh(4 * s)));
    // A third worker steals the tail of segment 0, inserting
    // a segment between the two in-flight ones.
    assert!(matches!(p.claim(), Some(Claim::Steal { .. })));
    p.progress(4 * s, 40);
    assert_eq!(
        p.segment(4 * s).map(|x| x.done),
        Some(40),
        "progress lands on the segment that STARTS at 4*s"
    );
    assert_eq!(p.downloaded(), 40);
}

#[test]
fn sanitize_reserved_stem_ignores_trailing_space_and_dots() {
    // AUDIT 0.32.1 (P1-17): Windows resolves "CON .txt" to the
    // CON device — the stem check must trim the same trailing
    // spaces/dots Win32 trims before comparing.
    assert_eq!(sanitize_file_name("CON .txt"), "_CON .txt");
    assert_eq!(sanitize_file_name("aux .log"), "_aux .log");
    assert_eq!(sanitize_file_name("CONIN$"), "_CONIN$");
    assert_eq!(sanitize_file_name("conout$.txt"), "_conout$.txt");
    assert_eq!(
        sanitize_file_name("console .txt"),
        "console .txt",
        "a merely similar stem stays untouched"
    );
}

#[test]
fn sanitize_cap_counts_utf16_units_not_just_chars() {
    // AUDIT 0.32.1 (P1-17): the cap counted chars(); 200
    // astral chars are 400 UTF-16 code units — over NTFS's
    // 255-unit component limit, so the create failed.
    let astral = "𝄞".repeat(200);
    let out = sanitize_file_name(&astral);
    assert!(
        out.encode_utf16().count() <= 200,
        "utf16 units: {}",
        out.encode_utf16().count()
    );
    assert!(out.chars().count() <= 200);
    // Extension still preserved when the stem is astral.
    let with_ext = format!("{}.mkv", "𝄞".repeat(150));
    let out = sanitize_file_name(&with_ext);
    assert!(out.ends_with(".mkv"));
    assert!(out.encode_utf16().count() <= 200);
}

#[test]
fn dedupe_suffix_stays_within_the_cap() {
    // AUDIT 0.32.1 (P1-17): the " (n)" suffix used to be
    // appended past the sanitizer's cap, producing names the
    // filesystem refuses.
    let name = format!("{}.txt", "a".repeat(196)); // exactly 200 chars
    assert_eq!(name.chars().count(), 200);
    let out = dedupe_file_name(&name, |n| n == name);
    assert_eq!(out.chars().count(), 200, "candidate within cap: {out}");
    assert!(out.ends_with(" (1).txt"), "{out}");
}

#[test]
fn dedupe_finds_a_free_name_past_1000() {
    // AUDIT 0.32.1 (P1-23): the old cap returned "a (999).txt"
    // EVEN THOUGH IT EXISTED when 1..=999 were all taken —
    // and the finalize rename on Windows replaces, so the
    // "never overwrites" promise broke exactly there.
    let out = dedupe_file_name("a.txt", |n| !n.ends_with("(1001).txt"));
    assert_eq!(out, "a (1001).txt");
}

#[test]
fn cookie_header_respects_the_cookie_path() {
    // AUDIT 0.32.1 (P2-3): the Path scope used to be ignored
    // when the header was recomputed per redirect hop, so a
    // path-scoped session cookie leaked to sibling paths (and
    // subdomains) the browser itself would never send it to.
    let mut c = cookie("sid", "example.com", false, false);
    c.path = "/private".to_string();
    let cs = vec![c];
    assert_eq!(
        cookie_header(&cs, "https://example.com/private/file").as_deref(),
        Some("sid=v")
    );
    assert_eq!(
        cookie_header(&cs, "https://example.com/private").as_deref(),
        Some("sid=v")
    );
    assert_eq!(cookie_header(&cs, "https://example.com/public/file"), None);
    assert_eq!(
        cookie_header(&cs, "https://example.com/privatex"),
        None,
        "a bare prefix without the slash boundary does not match"
    );
    // "/" (and an empty path) match everything.
    let cs = vec![cookie("a", "example.com", false, false)];
    assert_eq!(
        cookie_header(&cs, "https://example.com/anywhere/deep").as_deref(),
        Some("a=v")
    );
}

#[test]
fn cookie_header_drops_expired_cookies() {
    // AUDIT 0.32.1 (P2-4): captured cookies carried no expiry,
    // so a cookie the browser later expired or the user
    // deleted stayed valid in the task file forever.
    let mut dead = cookie("old", "example.com", false, false);
    dead.expires_unix_ms = Some(1); // 1970: long past
    let mut live = cookie("new", "example.com", false, false);
    live.expires_unix_ms = Some(u64::MAX / 2); // effectively never
    let cs = vec![dead, live];
    assert_eq!(
        cookie_header(&cs, "https://example.com/f").as_deref(),
        Some("new=v")
    );
    // And sanitization drops the expired one from the
    // persisted state entirely.
    let cleaned = sanitize_cookies(cs);
    assert_eq!(cleaned.len(), 1);
    assert_eq!(cleaned[0].name, "new");
}

#[test]
fn sanitize_cookies_drops_header_injection_shapes() {
    // AUDIT 0.32.1 (P1-19): a ';' in a value would be emitted
    // verbatim into the Cookie header, minting extra pairs; a
    // ';' or '=' in a name does the same. Control stripping
    // alone (the old rule) lets all three through.
    let mut bad_value = cookie("a", "example.com", false, false);
    bad_value.value = "x; injected=1".to_string();
    let mut bad_name = cookie("a;b", "example.com", false, false);
    bad_name.value = "v".to_string();
    let mut bad_name2 = cookie("a=b", "example.com", false, false);
    bad_name2.value = "v".to_string();
    let good = cookie("ok", "example.com", false, false);
    let cleaned = sanitize_cookies(vec![bad_value, bad_name, bad_name2, good]);
    assert_eq!(cleaned.len(), 1);
    assert_eq!(cleaned[0].name, "ok");
}

#[test]
fn cookie_capture_is_allowed_only_for_related_hosts() {
    // AUDIT 0.32.1 (P2-1): cookie capture must be scoped to
    // the INITIATING PAGE's site, not just to the download
    // URL — otherwise any page can make Pico fire an
    // authenticated GET at an arbitrary host (blind CSRF by
    // download) using the profile's cookies for that host.
    let page = "https://example.com/account";
    assert!(cookie_capture_allowed(page, "https://example.com/f.zip"));
    assert!(cookie_capture_allowed(
        page,
        "https://cdn.example.com/f.zip"
    ));
    assert!(cookie_capture_allowed(
        "https://www.example.com/",
        "https://example.com/f.zip"
    ));
    assert!(!cookie_capture_allowed(
        "https://evil.example/",
        "https://example.com/export"
    ));
    assert!(!cookie_capture_allowed(
        page,
        "https://notexample.com/f.zip"
    ));
    assert!(!cookie_capture_allowed(
        page,
        "https://example.com.evil.net/f.zip"
    ));
    // No page / no host: no capture.
    assert!(!cookie_capture_allowed(
        "about:blank",
        "https://example.com/f"
    ));
}

#[test]
fn strip_url_userinfo_removes_credentials_only() {
    // AUDIT 0.32.1 (P2-4): the task URL was persisted
    // verbatim; an embedded password sat in the task JSON for
    // the task's whole life.
    assert_eq!(
        strip_url_userinfo("https://user:pass@files.example.com:8443/x?q=1"),
        "https://files.example.com:8443/x?q=1"
    );
    assert_eq!(
        strip_url_userinfo("https://user@h.example/f"),
        "https://h.example/f"
    );
    assert_eq!(
        strip_url_userinfo("https://h.example/p@th"),
        "https://h.example/p@th",
        "an @ in the path is not userinfo"
    );
    assert_eq!(
        strip_url_userinfo("https://h.example/f"),
        "https://h.example/f"
    );
}

#[test]
fn cookie_expiry_converts_webview_dates() {
    // AUDIT 0.32.1 (P2-4): ICoreWebView2Cookie::Expires is an
    // OLE DATE (days since 1899-12-30); session cookies read
    // as <= 0 and must become None, not a 19th-century expiry.
    assert_eq!(cookie_expiry_unix_ms(-1.0), None);
    assert_eq!(cookie_expiry_unix_ms(0.0), None);
    assert_eq!(cookie_expiry_unix_ms(f64::NAN), None);
    // 2026-01-01T00:00:00Z = DATE 46023.0 = 1767225600000 ms.
    assert_eq!(cookie_expiry_unix_ms(46023.0), Some(1_767_225_600_000));
}

#[test]
fn sanitized_drops_empty_validators_and_strips_url_userinfo() {
    // AUDIT 0.32.1 (P1-5): an empty-string ETag in a task file
    // was never normalized, so Some("") vs a probe None fell
    // through to total-only resume instead of comparing as
    // "no validator".
    let mut t = task(Some(1_000), plan_segments(1_000, 2));
    t.etag = Some(String::new());
    t.last_modified = Some("   ".to_string());
    t.url = "https://user:pass@files.example.com/movie.mkv".to_string();
    let clean = t.sanitized().expect("recoverable");
    assert_eq!(clean.etag, None);
    assert_eq!(clean.last_modified, None);
    assert_eq!(clean.url, "https://files.example.com/movie.mkv");
}

#[test]
fn load_tasks_dedupes_ids_and_names_and_parks_queued_tasks() {
    // AUDIT 0.32.1 (P1-20 / P4-A3 / P4-13): duplicate ids in
    // hand-planted task files used to alias two rows onto one
    // task; duplicate file names made two tasks share one
    // .part; and a task persisted as Queued loaded as Queued
    // with no supervisor and no way to resume it — stuck
    // forever. Queued now parks as Paused, like Downloading.
    let dir = tmp("dupes");
    let write = |file: &str, t: &TaskState| {
        fs::write(dir.join(file), serde_json::to_string(t).expect("serialize"))
            .expect("write task file");
    };
    let mut a = task(Some(10), plan_segments(10, 1));
    a.id = 21;
    a.status = TaskStatus::Queued;
    write("a.json", &a);
    let mut b = task(Some(10), plan_segments(10, 1));
    b.id = 21; // same id, different file
    b.file_name = "other.mkv".to_string();
    write("b.json", &b);
    let mut c = task(Some(10), plan_segments(10, 1));
    c.id = 22;
    c.file_name = a.file_name.clone(); // same name as task 21
    write("c.json", &c);
    let loaded = load_tasks(&dir);
    assert_eq!(loaded.len(), 2, "the duplicate id is dropped: {loaded:?}");
    let t21 = loaded.iter().find(|t| t.id == 21).expect("task 21");
    assert_eq!(
        t21.status,
        TaskStatus::Paused,
        "a Queued task loads parked as Paused"
    );
    let t22 = loaded.iter().find(|t| t.id == 22).expect("task 22");
    assert_ne!(
        t22.file_name, t21.file_name,
        "two live tasks never share one .part name"
    );
    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// 0.32.2: download URI triage + run generations
// ---------------------------------------------------------------------------

#[test]
fn triage_accepts_http_and_https_with_a_host() {
    // AUDIT 0.32.2 (Fix 1): acceptance is exactly
    // is_downloadable_url — an https URL with a host, and an
    // http URL carrying userinfo and a port, both accept.
    assert_eq!(
        triage_download_uri("https://example.com/file.zip"),
        DownloadTriage::Accept
    );
    assert_eq!(
        triage_download_uri("http://user@example.com:8080/file.zip"),
        DownloadTriage::Accept
    );
}

#[test]
fn triage_rejects_with_plain_user_facing_reasons() {
    // AUDIT 0.32.2 (Fix 1): every refusal names WHY, in words
    // the panel's error line can show verbatim. The scheme is
    // extracted before the "://" check, so blob: (which
    // contains "://" inside itself) lands on the scheme
    // branch, not the no-scheme branch.
    assert_eq!(
        triage_download_uri(""),
        DownloadTriage::Reject("empty link")
    );
    assert_eq!(
        triage_download_uri("   "),
        DownloadTriage::Reject("empty link")
    );
    assert_eq!(
        triage_download_uri("example.com/file"),
        DownloadTriage::Reject("not a web link")
    );
    assert_eq!(
        triage_download_uri("https:///path"),
        DownloadTriage::Reject("the link has no host")
    );
    let only = DownloadTriage::Reject("Pico can only download http/https links");
    assert_eq!(triage_download_uri("blob:https://x/1"), only);
    assert_eq!(triage_download_uri("data:text/plain;base64,AA=="), only);
    assert_eq!(triage_download_uri("file:///C:/x"), only);
    assert_eq!(triage_download_uri("ftp://h/f"), only);
}

#[test]
fn generation_gate_only_accepts_the_current_token() {
    // AUDIT 0.32.2 (Fix 2): token 0 is never valid, and a
    // fresh gate (current 0) accepts nothing at all.
    let mut gate = GenerationGate::new();
    assert_eq!(gate.current(), 0);
    assert!(!gate.accepts(0));
    assert!(!gate.accepts(1));

    // begin mints the live token.
    let first = gate.begin();
    assert!(first > 0, "begin never hands out the invalid 0");
    assert!(gate.accepts(first));
    assert!(!gate.accepts(0), "0 stays invalid after begins");

    // invalidate retires the live token.
    let after_invalidate = gate.invalidate();
    assert!(after_invalidate > first, "tokens strictly increase");
    assert!(!gate.accepts(first), "the old token is rejected");
    assert_eq!(gate.current(), after_invalidate);

    // A second begin retires the first token too.
    let second = gate.begin();
    assert!(second > after_invalidate, "tokens strictly increase");
    assert!(!gate.accepts(first));
    assert!(!gate.accepts(after_invalidate));
    assert!(gate.accepts(second));
    assert_eq!(gate.current(), second);
}

// -----------------------------------------------------------------
// The captured-navigation unwind (0.32.3): the pure
// decision over tab state. The defect it closes: a
// main-frame navigation to a file URL is adopted into the
// tab's live URL, the download is captured and cancelled
// by the handler, and NO load-finish ever reconciles the
// tab — it shows Loading on the file URL forever. The
// unwind restores the tab, gated on the tab still being
// on the captured URL.
// -----------------------------------------------------------------

use pico_core::core::download::{unwind_action, UnwindAction, UnwindOrigin};

const FILE: &str = "https://dl2.example.com/soft/s/tool.rar?1794333393";
const PAGE: &str = "https://example.com/download/tool/";
const OLDER: &str = "https://example.com/";

/// The no-redirect chain (0.35, WS2): a navigation that
/// never redirected occupies exactly one URL — the file
/// itself. Every pre-0.35 matrix case above/below is
/// re-pointed at the chain gate with this singleton,
/// which reproduces the old exact-match gate exactly.
fn test_chain() -> Vec<String> {
    vec![FILE.to_string()]
}

/// Redirect hops for the 0.35 chain cases: the tab's
/// live URL sits on HOP0 while the download handler
/// hands over FILE (the post-redirect final URL).
const HOP0: &str = "https://dl2.example.com/get/tool.rar";
const HOP1: &str = "https://mirror.example.com/hop/tool.rar";

fn redirect_chain() -> Vec<String> {
    vec![HOP0.to_string(), HOP1.to_string(), FILE.to_string()]
}

#[test]
fn unwind_reloads_the_committed_download_page() {
    // The user's exact shape: the tab sits on the file
    // URL; its last committed page is the download page
    // he clicked from.
    assert_eq!(
        unwind_action(
            FILE,
            Some(PAGE),
            Some(OLDER),
            FILE,
            &test_chain(),
            false,
            UnwindOrigin::User,
            false
        ),
        UnwindAction::NavigateTo(PAGE.to_string())
    );
}

#[test]
fn unwind_none_when_the_tab_moved_on() {
    // He navigated away himself between the click and the
    // capture landing: never yank the tab back.
    assert_eq!(
        unwind_action(
            PAGE,
            Some(PAGE),
            Some(OLDER),
            FILE,
            &test_chain(),
            false,
            UnwindOrigin::User,
            false
        ),
        UnwindAction::None
    );
    // A third page in the address bar: equally hands-off,
    // even if the committed entry somehow names the file.
    assert_eq!(
        unwind_action(
            OLDER,
            Some(FILE),
            Some(PAGE),
            FILE,
            &test_chain(),
            false,
            UnwindOrigin::User,
            false
        ),
        UnwindAction::None
    );
}

#[test]
fn unwind_none_for_captures_that_never_navigated() {
    // A download-attribute / subframe capture: the tab's
    // live URL is its page, not the file — nothing wedged.
    assert_eq!(
        unwind_action(
            PAGE,
            Some(PAGE),
            Some(OLDER),
            FILE,
            &test_chain(),
            false,
            UnwindOrigin::User,
            false
        ),
        UnwindAction::None
    );
}

#[test]
fn unwind_none_during_a_pending_traversal() {
    // Back/Forward is already in flight toward some entry:
    // the landing reconciles the tab; an unwind on top
    // would race it.
    assert_eq!(
        unwind_action(
            FILE,
            Some(PAGE),
            Some(OLDER),
            FILE,
            &test_chain(),
            true,
            UnwindOrigin::User,
            false
        ),
        UnwindAction::None
    );
}

#[test]
fn unwind_back_when_the_file_itself_was_committed() {
    // The file URL reached the history (a fast finish
    // before the cancel): a real traversal back to the
    // page before it, whose landing reconciles everything.
    assert_eq!(
        unwind_action(
            FILE,
            Some(FILE),
            Some(PAGE),
            FILE,
            &test_chain(),
            false,
            UnwindOrigin::User,
            false
        ),
        UnwindAction::Back
    );
}

#[test]
fn unwind_start_page_when_no_page_survives() {
    // Committed file, nothing before it: the user-origin
    // case — a fresh user tab whose only navigation was
    // the file (the adopted variants get their own test
    // below and close instead).
    assert_eq!(
        unwind_action(
            FILE,
            Some(FILE),
            None,
            FILE,
            &test_chain(),
            false,
            UnwindOrigin::User,
            false
        ),
        UnwindAction::StartPage
    );
    // Committed file, and the entry before it is ALSO the
    // file URL: going back would land on the file again
    // (and re-capture) — start page instead.
    assert_eq!(
        unwind_action(
            FILE,
            Some(FILE),
            Some(FILE),
            FILE,
            &test_chain(),
            false,
            UnwindOrigin::User,
            false
        ),
        UnwindAction::StartPage
    );
    // Nothing committed at all, but the live URL is the
    // file: the tab never had a page.
    assert_eq!(
        unwind_action(
            FILE,
            None,
            None,
            FILE,
            &test_chain(),
            false,
            UnwindOrigin::User,
            false
        ),
        UnwindAction::StartPage
    );
}

#[test]
fn unwind_none_for_an_empty_capture() {
    // An empty URI names no navigation: nothing to match,
    // nothing to unwind.
    assert_eq!(
        unwind_action(
            FILE,
            Some(PAGE),
            Some(OLDER),
            "",
            &test_chain(),
            false,
            UnwindOrigin::User,
            false
        ),
        UnwindAction::None
    );
}

#[test]
fn unwind_is_consumed_by_its_own_restore() {
    // After the unwind the tab's live URL is the restored
    // page, so a second evaluation of the same capture
    // (a duplicate event, a late arrival) decides None —
    // the unwind cannot loop.
    assert_eq!(
        unwind_action(
            PAGE,
            Some(PAGE),
            Some(OLDER),
            FILE,
            &test_chain(),
            false,
            UnwindOrigin::User,
            false
        ),
        UnwindAction::None
    );
}

#[test]
fn unwind_closes_an_adopted_tab_that_only_carried_the_file() {
    // Adopted, not the only tab, nothing committed: the
    // site opened this tab only to carry the download.
    assert_eq!(
        unwind_action(
            FILE,
            None,
            None,
            FILE,
            &test_chain(),
            false,
            UnwindOrigin::Adopted,
            false
        ),
        UnwindAction::CloseTab
    );
    // Adopted, committed file, nothing before it: same
    // download-only tab, the file just happened to commit.
    assert_eq!(
        unwind_action(
            FILE,
            Some(FILE),
            None,
            FILE,
            &test_chain(),
            false,
            UnwindOrigin::Adopted,
            false
        ),
        UnwindAction::CloseTab
    );
    // Adopted, committed file, and the entry before it is
    // also the file: going back would loop, so it closes.
    assert_eq!(
        unwind_action(
            FILE,
            Some(FILE),
            Some(FILE),
            FILE,
            &test_chain(),
            false,
            UnwindOrigin::Adopted,
            false
        ),
        UnwindAction::CloseTab
    );
}

#[test]
fn unwind_never_closes_the_only_tab_or_a_users_tab() {
    // Adopted but the window's only tab: a window never
    // ends with zero tabs — start page instead.
    assert_eq!(
        unwind_action(
            FILE,
            None,
            None,
            FILE,
            &test_chain(),
            false,
            UnwindOrigin::Adopted,
            true
        ),
        UnwindAction::StartPage
    );
    // A user's own tab, not the only tab, nothing
    // committed: never closed — start page instead.
    assert_eq!(
        unwind_action(
            FILE,
            None,
            None,
            FILE,
            &test_chain(),
            false,
            UnwindOrigin::User,
            false
        ),
        UnwindAction::StartPage
    );
}

#[test]
fn unwind_adopted_tab_with_a_real_page_still_restores_it() {
    // Adopted, committed file, a real page before it: the
    // tab had a life before the file — go Back.
    assert_eq!(
        unwind_action(
            FILE,
            Some(FILE),
            Some(PAGE),
            FILE,
            &test_chain(),
            false,
            UnwindOrigin::Adopted,
            false
        ),
        UnwindAction::Back
    );
    // Adopted, a real page committed: reload that page,
    // origin does not change this case.
    assert_eq!(
        unwind_action(
            FILE,
            Some(PAGE),
            Some(OLDER),
            FILE,
            &test_chain(),
            false,
            UnwindOrigin::Adopted,
            false
        ),
        UnwindAction::NavigateTo(PAGE.to_string())
    );
}

#[test]
fn unwind_gates_dominate_origin() {
    // Adopted, but the tab has moved off the captured
    // URL: the live-URL gate decides None first.
    assert_eq!(
        unwind_action(
            PAGE,
            None,
            None,
            FILE,
            &test_chain(),
            false,
            UnwindOrigin::Adopted,
            false
        ),
        UnwindAction::None
    );
    // Adopted, but a traversal is already pending: that
    // gate decides None first too.
    assert_eq!(
        unwind_action(
            FILE,
            None,
            None,
            FILE,
            &test_chain(),
            true,
            UnwindOrigin::Adopted,
            false
        ),
        UnwindAction::None
    );
}

// ---------------------------------------------------------------------------
// 0.35, WS2: the chain gate. The defect: his flow is
// page tab → site opens a NEW tab at a pre-redirect URL
// → 302 → CDN host → file. The tab's live URL holds the
// PRE-redirect hop while the download handler hands
// over the POST-redirect URI, so the pre-0.35 gate
// (live == captured, exact string) could never pass and
// the fate silently decided None. The capture now
// carries the whole chain and the gate is live ∈ chain.
// ---------------------------------------------------------------------------

#[test]
fn unwind_chain_gate_closes_an_adopted_tab_through_redirects() {
    // His exact shape: live = hop0, captured = final,
    // chain = [hop0, hop1, final], nothing committed.
    assert_eq!(
        unwind_action(
            HOP0,
            None,
            None,
            FILE,
            &redirect_chain(),
            false,
            UnwindOrigin::Adopted,
            false
        ),
        UnwindAction::CloseTab
    );
    // Any hop as the live URL passes the gate — the
    // mirror direction (the tab settled on a later hop)
    // closes the same way.
    assert_eq!(
        unwind_action(
            HOP1,
            None,
            None,
            FILE,
            &redirect_chain(),
            false,
            UnwindOrigin::Adopted,
            false
        ),
        UnwindAction::CloseTab
    );
    // A user's own tab on the same chain still never
    // closes: start page instead (origin is judged
    // exactly as before, past the widened gate).
    assert_eq!(
        unwind_action(
            HOP0,
            None,
            None,
            FILE,
            &redirect_chain(),
            false,
            UnwindOrigin::User,
            false
        ),
        UnwindAction::StartPage
    );
    // And the only tab on the chain converts, never closes.
    assert_eq!(
        unwind_action(
            HOP0,
            None,
            None,
            FILE,
            &redirect_chain(),
            false,
            UnwindOrigin::Adopted,
            true
        ),
        UnwindAction::StartPage
    );
}

#[test]
fn unwind_chain_gate_still_refuses_a_genuine_navigation() {
    // The invariant: a URL outside the chain is a
    // navigation the user made themselves — hands off,
    // however adopted the tab is.
    assert_eq!(
        unwind_action(
            PAGE,
            None,
            None,
            FILE,
            &redirect_chain(),
            false,
            UnwindOrigin::Adopted,
            false
        ),
        UnwindAction::None
    );
    // A traversal pending on a chain hop: the second
    // gate still dominates.
    assert_eq!(
        unwind_action(
            HOP0,
            None,
            None,
            FILE,
            &redirect_chain(),
            true,
            UnwindOrigin::Adopted,
            false
        ),
        UnwindAction::None
    );
}

#[test]
fn unwind_chain_members_are_dropped_from_committed_and_prev() {
    // A chain hop committed as the current entry, with a
    // real page before it: the tab had a life before the
    // chain — go Back to that page (the hop itself is
    // not a page to reload).
    assert_eq!(
        unwind_action(
            HOP0,
            Some(HOP1),
            Some(PAGE),
            FILE,
            &redirect_chain(),
            false,
            UnwindOrigin::User,
            false
        ),
        UnwindAction::Back
    );
    // The final URL committed, and the entry before it
    // is another hop: the hop is dropped before judging,
    // so there is no real previous entry to go Back to —
    // an adopted download-only tab closes instead of
    // backing onto a redirect hop.
    assert_eq!(
        unwind_action(
            HOP0,
            Some(FILE),
            Some(HOP1),
            FILE,
            &redirect_chain(),
            false,
            UnwindOrigin::Adopted,
            false
        ),
        UnwindAction::CloseTab
    );
    // A real page committed, with a hop behind it: the
    // page reloads (Restore is judged on the committed
    // entry; the in-chain prev is dropped and changes
    // nothing).
    assert_eq!(
        unwind_action(
            HOP0,
            Some(PAGE),
            Some(HOP1),
            FILE,
            &redirect_chain(),
            false,
            UnwindOrigin::User,
            false
        ),
        UnwindAction::NavigateTo(PAGE.to_string())
    );
}

// ---------------------------------------------------------------------------
// 0.33: failure taxonomy, retry policy, hints
// ---------------------------------------------------------------------------

#[test]
fn http_status_of_finds_the_status_token() {
    assert_eq!(http_status_of("HTTP 404"), Some(404));
    assert_eq!(http_status_of("HTTP 416"), Some(416));
    assert_eq!(
        http_status_of("unfollowable redirect (HTTP 302)"),
        Some(302)
    );
    assert_eq!(http_status_of("short response (HTTP 206)"), Some(206));
    assert_eq!(http_status_of("range mismatch (HTTP 206)"), Some(206));
    assert_eq!(http_status_of("HTTP 500"), Some(500));
    // No token, a short token, and a long digit run are not statuses.
    assert_eq!(http_status_of("connection reset by peer"), None);
    assert_eq!(http_status_of("HTTP 44"), None);
    assert_eq!(http_status_of("HTTP 4040"), None);
    assert_eq!(http_status_of(""), None);
}

#[test]
fn classify_failure_maps_statuses() {
    assert_eq!(classify_failure("HTTP 404"), FailureClass::NotFound);
    assert_eq!(classify_failure("HTTP 401"), FailureClass::Auth);
    assert_eq!(classify_failure("HTTP 403"), FailureClass::Auth);
    assert_eq!(classify_failure("HTTP 429"), FailureClass::RateLimited);
    assert_eq!(classify_failure("HTTP 500"), FailureClass::Server);
    assert_eq!(classify_failure("HTTP 503"), FailureClass::Server);
    assert_eq!(classify_failure("HTTP 599"), FailureClass::Server);
    assert_eq!(classify_failure("HTTP 416"), FailureClass::RangeLost);
    // Any other status is Other — including a redirect the
    // transport could not follow and a 206 short response.
    assert_eq!(
        classify_failure("unfollowable redirect (HTTP 302)"),
        FailureClass::Other
    );
    assert_eq!(classify_failure("HTTP 400"), FailureClass::Other);
    assert_eq!(
        classify_failure("short response (HTTP 206)"),
        FailureClass::Other
    );
    // Labels are the stable wire spellings.
    assert_eq!(FailureClass::Network.label(), "network");
    assert_eq!(FailureClass::Server.label(), "server");
    assert_eq!(FailureClass::RateLimited.label(), "rate-limited");
    assert_eq!(FailureClass::NotFound.label(), "not-found");
    assert_eq!(FailureClass::Auth.label(), "auth");
    assert_eq!(FailureClass::Disk.label(), "disk");
    assert_eq!(FailureClass::RangeLost.label(), "range-lost");
    assert_eq!(FailureClass::Changed.label(), "changed");
    assert_eq!(FailureClass::Other.label(), "other");
}

#[test]
fn classify_failure_maps_the_managers_message_shapes() {
    // Disk family (the manager's real strings).
    assert_eq!(
        classify_failure("not enough disk space: the file needs 1 MB more, 0 B free"),
        FailureClass::Disk
    );
    assert_eq!(
        classify_failure("disk nearly full — download stopped"),
        FailureClass::Disk
    );
    assert_eq!(
        classify_failure("cannot prepare file: access denied"),
        FailureClass::Disk
    );
    assert_eq!(
        classify_failure("cannot open file: not found"),
        FailureClass::Disk
    );
    assert_eq!(
        classify_failure("cannot move finished file: access denied"),
        FailureClass::Disk
    );
    assert_eq!(
        classify_failure("write failed: disk error"),
        FailureClass::Disk
    );
    assert_eq!(
        classify_failure("seek failed: disk error"),
        FailureClass::Disk
    );
    // Range / changed / other shapes.
    assert_eq!(
        classify_failure("server stopped honoring ranges"),
        FailureClass::RangeLost
    );
    assert_eq!(classify_failure("size mismatch"), FailureClass::Changed);
    assert_eq!(classify_failure("unsupported URL"), FailureClass::Other);
    assert_eq!(
        classify_failure("winhttp transport unavailable on this platform"),
        FailureClass::Other
    );
    // Everything unrecognized is a network failure.
    assert_eq!(
        classify_failure("connection reset by peer"),
        FailureClass::Network
    );
    assert_eq!(classify_failure("timed out"), FailureClass::Network);
    assert_eq!(classify_failure(""), FailureClass::Network);
}

#[test]
fn retry_policy_follows_the_backoff_ladder_and_the_cap() {
    // Backoff order is 5s / 15s / 45s for retries 0 / 1 / 2.
    assert_eq!(AUTO_RETRY_BACKOFF_MS, [5_000, 15_000, 45_000]);
    assert_eq!(AUTO_RETRY_MAX, 3);
    for class in [
        FailureClass::Network,
        FailureClass::Server,
        FailureClass::RateLimited,
    ] {
        assert_eq!(
            retry_policy(class, 0),
            RetryPolicy::Retry { after_ms: 5_000 },
            "{class:?} retry 0"
        );
        assert_eq!(
            retry_policy(class, 1),
            RetryPolicy::Retry { after_ms: 15_000 },
            "{class:?} retry 1"
        );
        assert_eq!(
            retry_policy(class, 2),
            RetryPolicy::Retry { after_ms: 45_000 },
            "{class:?} retry 2"
        );
        // At the cap the ladder ends.
        assert_eq!(
            retry_policy(class, 3),
            RetryPolicy::FailNow,
            "{class:?} capped"
        );
        assert_eq!(
            retry_policy(class, 99),
            RetryPolicy::FailNow,
            "{class:?} past cap"
        );
    }
}

#[test]
fn retry_policy_restarts_range_and_change_exactly_once() {
    for class in [FailureClass::RangeLost, FailureClass::Changed] {
        assert_eq!(
            retry_policy(class, 0),
            RetryPolicy::RestartOnce,
            "{class:?} at 0"
        );
        assert_eq!(
            retry_policy(class, 1),
            RetryPolicy::FailNow,
            "{class:?} at 1"
        );
        assert_eq!(
            retry_policy(class, 3),
            RetryPolicy::FailNow,
            "{class:?} at cap"
        );
    }
    for class in [
        FailureClass::NotFound,
        FailureClass::Auth,
        FailureClass::Disk,
        FailureClass::Other,
    ] {
        assert_eq!(
            retry_policy(class, 0),
            RetryPolicy::FailNow,
            "{class:?} never retries"
        );
    }
}

#[test]
fn failure_hint_exists_only_where_it_adds_information() {
    assert_eq!(
        failure_hint(FailureClass::Auth),
        Some("the link may have expired — use Refresh link")
    );
    assert_eq!(
        failure_hint(FailureClass::NotFound),
        Some("the file is gone from the server (404)")
    );
    assert_eq!(
        failure_hint(FailureClass::Disk),
        Some("a disk problem stopped this download — free space, then Resume")
    );
    assert_eq!(
        failure_hint(FailureClass::RangeLost),
        Some("the server stopped honoring ranges")
    );
    assert_eq!(
        failure_hint(FailureClass::Changed),
        Some("the file changed on the server")
    );
    for class in [
        FailureClass::Network,
        FailureClass::Server,
        FailureClass::RateLimited,
        FailureClass::Other,
    ] {
        assert_eq!(failure_hint(class), None, "{class:?} has no hint");
    }
}

// ---------------------------------------------------------------------------
// 0.33: refresh_decision
// ---------------------------------------------------------------------------

fn refresh_probe() -> ProbeFacts {
    ProbeFacts {
        status: 200,
        total: Some(1_000),
        accept_ranges: true,
        etag: None,
        last_modified: None,
    }
}

#[test]
fn refresh_resumes_when_every_fact_agrees() {
    let t = downloaded_task();
    assert_eq!(
        refresh_decision(&t, &refresh_probe()),
        RefreshDecision::RebindResume
    );
    // A redirect-range status still resumes (200..400).
    let mut p = refresh_probe();
    p.status = 302;
    assert_eq!(refresh_decision(&t, &p), RefreshDecision::RebindResume);
    // Matching validators on both sides keep the resume.
    let mut t2 = downloaded_task();
    t2.etag = Some("\"v1\"".to_string());
    t2.last_modified = Some("Mon, 01 Jan 2026 00:00:00 GMT".to_string());
    let mut p2 = refresh_probe();
    p2.etag = Some("\"v1\"".to_string());
    p2.last_modified = Some("Mon, 01 Jan 2026 00:00:00 GMT".to_string());
    assert_eq!(refresh_decision(&t2, &p2), RefreshDecision::RebindResume);
    // A validator present on one side only is not a contradiction.
    let mut p3 = refresh_probe();
    p3.etag = Some("\"anything\"".to_string());
    assert_eq!(refresh_decision(&t, &p3), RefreshDecision::RebindResume);
}

#[test]
fn refresh_restarts_on_each_single_disagreement() {
    let t = downloaded_task();
    // Bad status (below the range, and at/above its top).
    let mut p = refresh_probe();
    p.status = 404;
    assert_eq!(refresh_decision(&t, &p), RefreshDecision::RebindRestart);
    let mut p = refresh_probe();
    p.status = 400;
    assert_eq!(refresh_decision(&t, &p), RefreshDecision::RebindRestart);
    // Zero downloaded: nothing to resume.
    let fresh = task(Some(1_000), plan_segments(1_000, 2));
    assert_eq!(
        refresh_decision(&fresh, &refresh_probe()),
        RefreshDecision::RebindRestart
    );
    // Total mismatch.
    let mut p = refresh_probe();
    p.total = Some(2_000);
    assert_eq!(refresh_decision(&t, &p), RefreshDecision::RebindRestart);
    // Total unknown on the probe side.
    let mut p = refresh_probe();
    p.total = None;
    assert_eq!(refresh_decision(&t, &p), RefreshDecision::RebindRestart);
    // Total unknown on the saved side.
    let mut t_none = downloaded_task();
    t_none.total = None;
    assert_eq!(
        refresh_decision(&t_none, &refresh_probe()),
        RefreshDecision::RebindRestart
    );
    // ETag contradicted.
    let mut t_etag = downloaded_task();
    t_etag.etag = Some("\"v1\"".to_string());
    let mut p = refresh_probe();
    p.etag = Some("\"v2\"".to_string());
    assert_eq!(
        refresh_decision(&t_etag, &p),
        RefreshDecision::RebindRestart
    );
    // Last-Modified contradicted.
    let mut t_lm = downloaded_task();
    t_lm.last_modified = Some("Mon, 01 Jan 2026 00:00:00 GMT".to_string());
    let mut p = refresh_probe();
    p.last_modified = Some("Tue, 02 Jan 2026 00:00:00 GMT".to_string());
    assert_eq!(refresh_decision(&t_lm, &p), RefreshDecision::RebindRestart);
    // Ranges lost.
    let mut p = refresh_probe();
    p.accept_ranges = false;
    assert_eq!(refresh_decision(&t, &p), RefreshDecision::RebindRestart);
}

// ---------------------------------------------------------------------------
// 0.33: TokenBucket
// ---------------------------------------------------------------------------

#[test]
fn token_bucket_starts_full_and_grants_the_full_burst() {
    let mut b = TokenBucket::new(1_000, 0);
    assert_eq!(b.rate(), 1_000);
    assert_eq!(b.available(0), 1_000);
    assert!(
        b.try_take(1_000, 0),
        "the full capacity can be taken at once"
    );
    assert_eq!(b.available(0), 0);
    assert!(!b.try_take(1, 0), "an empty bucket grants nothing");
}

#[test]
fn token_bucket_partial_wait_math_is_exact() {
    let mut b = TokenBucket::new(1_000, 0);
    assert!(b.try_take(1_000, 0));
    // Deficit 500 at 1000/s: exactly 500 ms.
    assert_eq!(b.wait_ms_for(500, 0), 500);
    // Deficit 1 at 1000/s: 1 ms (ceil, never 0 for a real wait).
    assert_eq!(b.wait_ms_for(1, 0), 1);
    // After half the wait, half the tokens are back.
    assert_eq!(b.available(500), 500);
    assert_eq!(b.wait_ms_for(500, 500), 0);
    assert!(b.try_take(500, 500));
    // Want 0 never waits; an affordable want never waits.
    let mut b = TokenBucket::new(100, 0);
    assert_eq!(b.wait_ms_for(0, 0), 0);
    assert_eq!(b.wait_ms_for(100, 0), 0);
    // Ceil: deficit 101 at 100/s = 1010 ms exactly; deficit
    // 1 at 3/s = ceil(1000/3) = 334 ms.
    let mut b = TokenBucket::new(3, 0);
    assert!(b.try_take(3, 0));
    assert_eq!(b.wait_ms_for(1, 0), 334);
}

#[test]
fn token_bucket_set_rate_refills_then_clamps() {
    let mut b = TokenBucket::new(1_000, 0);
    assert!(b.try_take(1_000, 0));
    // One second at the OLD rate refills to the old cap
    // (1000), then the new cap (100) clamps it down.
    b.set_rate(100, 1_000);
    assert_eq!(b.rate(), 100);
    assert_eq!(b.available(1_000), 100);
    // Raising the rate does not mint tokens: the balance
    // carries, only the capacity grows.
    b.set_rate(10_000, 1_000);
    assert_eq!(b.available(1_000), 100);
    // Setting rate 0 is unlimited from then on.
    b.set_rate(0, 1_000);
    assert!(b.try_take(u64::MAX / 2, 1_000));
    assert_eq!(b.wait_ms_for(1_000_000, 1_000), 0);
}

#[test]
fn token_bucket_rate_zero_is_unlimited() {
    let mut b = TokenBucket::new(0, 0);
    assert_eq!(b.rate(), 0);
    assert!(b.try_take(1_000_000_000, 0));
    assert!(b.try_take(1_000_000_000, 999_999));
    assert_eq!(b.wait_ms_for(1_000_000_000, 0), 0);
    assert_eq!(b.available(0), u64::MAX);
}

#[test]
fn token_bucket_many_small_refills_do_not_drift() {
    // 3 tokens/s refilled 1 ms at a time: each refill is
    // 0.003 tokens. A bucket that rounds each refill to
    // whole tokens would NEVER refill; the milli-token
    // balance reaches a full token after 334 ms.
    let mut b = TokenBucket::new(3, 0);
    assert!(b.try_take(3, 0));
    for t in 1..=333u64 {
        assert_eq!(b.available(t), 0, "t={t}: not yet a whole token");
    }
    assert_eq!(
        b.available(334),
        1,
        "334 x 3 milli-tokens = 1002 milli >= 1 token"
    );
    assert!(b.try_take(1, 334));
    // And over a full second the total granted equals the
    // rate exactly — no drift in either direction.
    let mut b = TokenBucket::new(1_000, 0);
    assert!(b.try_take(1_000, 0));
    let mut granted = 0u64;
    for t in 1..=1_000u64 {
        if b.try_take(1, t) {
            granted += 1;
        }
    }
    assert_eq!(granted, 1_000, "1000 x 1ms refills grant exactly the rate");
}

// ---------------------------------------------------------------------------
// 0.33: TaskState new fields — round-trip, old shape, sanitized
// ---------------------------------------------------------------------------

#[test]
fn task_state_new_fields_round_trip() {
    let mut t = task(Some(1_000), plan_segments(1_000, 2));
    t.referer = Some("https://example.com/download/page".to_string());
    t.sha256 = Some("a".repeat(64));
    t.auto_retries = 2;
    t.error_hint = Some("the file changed on the server".to_string());
    let text = serde_json::to_string(&t).expect("serialize");
    let back: TaskState = serde_json::from_str(&text).expect("deserialize");
    assert_eq!(back, t);
}

#[test]
fn task_state_old_shape_json_still_loads() {
    // A minimal pre-0.33 task file: no referer / sha256 /
    // auto_retries / error_hint keys at all. Serde defaults
    // fill them; sanitized() round-trips the result.
    let old = r#"{"version":1,"id":7,"url":"https://files.example.com/movie.mkv","file_name":"movie.mkv","total":1000,"etag":null,"last_modified":null,"connections":8,"status":"paused","added_ms":1000,"segments":[{"start":0,"end":1000,"done":0}],"cookies":[],"error":null}"#;
    let t: TaskState = serde_json::from_str(old).expect("old shape parses");
    assert_eq!(t.referer, None);
    assert_eq!(t.sha256, None);
    assert_eq!(t.auto_retries, 0);
    assert_eq!(t.error_hint, None);
    let clean = t.sanitized().expect("old shape sanitizes");
    assert_eq!(clean.version, TASK_FILE_VERSION);
    assert_eq!(clean.total, Some(1_000));
}

#[test]
fn sanitized_handles_the_new_fields() {
    // Referer: userinfo stripped, http/https kept.
    let mut t = task(Some(10), plan_segments(10, 1));
    t.referer = Some("https://user:pass@example.com/page".to_string());
    let clean = t.sanitized().expect("ok");
    assert_eq!(clean.referer.as_deref(), Some("https://example.com/page"));
    // Referer: a non-http(s) scheme is dropped.
    let mut t = task(Some(10), plan_segments(10, 1));
    t.referer = Some("file:///C:/page.html".to_string());
    assert_eq!(t.sanitized().expect("ok").referer, None);
    // Referer: over the 2048-char cap is dropped.
    let mut t = task(Some(10), plan_segments(10, 1));
    t.referer = Some(format!("https://example.com/{}", "a".repeat(2048)));
    assert_eq!(t.sanitized().expect("ok").referer, None);
    // Sha256: uppercase hex is kept, lowercased.
    let mut t = task(Some(10), plan_segments(10, 1));
    t.sha256 = Some("ABCDEF0123456789".repeat(4));
    let clean = t.sanitized().expect("ok");
    assert_eq!(clean.sha256, Some("abcdef0123456789".repeat(4)));
    // Sha256: junk shapes are dropped (too short, non-hex).
    let mut t = task(Some(10), plan_segments(10, 1));
    t.sha256 = Some("abc123".to_string());
    assert_eq!(t.sanitized().expect("ok").sha256, None);
    let mut t = task(Some(10), plan_segments(10, 1));
    t.sha256 = Some("z".repeat(64));
    assert_eq!(t.sanitized().expect("ok").sha256, None);
    // Auto-retries clamp to the cap; the hint caps at 200.
    let mut t = task(Some(10), plan_segments(10, 1));
    t.auto_retries = 99;
    t.error_hint = Some("h".repeat(500));
    let clean = t.sanitized().expect("ok");
    assert_eq!(clean.auto_retries, AUTO_RETRY_MAX);
    assert_eq!(
        clean.error_hint.as_ref().map(|h| h.chars().count()),
        Some(200)
    );
}

// ---------------------------------------------------------------------------
// 0.33: TabHistory::drop_top_entry + the WS1 composition
// ---------------------------------------------------------------------------

#[test]
fn drop_top_entry_single_entry_leaves_no_current() {
    let mut h = TabHistory::new();
    h.record_push("https://example.com/only");
    assert_eq!(h.current(), Some("https://example.com/only"));
    h.drop_top_entry();
    assert_eq!(h.current(), None);
    assert!(h.is_empty());
}

#[test]
fn drop_top_entry_two_entries_lands_on_the_previous() {
    let mut h = TabHistory::new();
    h.record_push("https://example.com/a");
    h.record_push("https://example.com/b");
    assert_eq!(h.current(), Some("https://example.com/b"));
    h.drop_top_entry();
    assert_eq!(h.current(), Some("https://example.com/a"));
    assert_eq!(h.len(), 1);
    assert!(!h.can_go_back());
}

#[test]
fn drop_top_entry_on_empty_history_is_a_noop() {
    let mut h = TabHistory::new();
    h.drop_top_entry();
    assert_eq!(h.current(), None);
    assert!(h.is_empty());
    // Still usable afterwards.
    h.record_push("https://example.com/x");
    assert_eq!(h.current(), Some("https://example.com/x"));
}

#[test]
fn ws1_repair_push_then_drop_restores_the_adopted_tab_to_no_page() {
    // The WS1 composition (0.33): the adopted download tab's
    // history is polluted by record_push(file) (pollution
    // arm 2 in the 0.33 contract). The repair drops that
    // entry; the tab is back to NO committed page, and the
    // unwind facts composed from the repaired state decide
    // CloseTab — the site-opened tab that only carried the
    // download closes instead of converting to StartPage.
    let mut h = TabHistory::new();
    h.record_push(FILE);
    h.drop_top_entry();
    assert_eq!(h.current(), None, "the file entry is gone");
    // Compose the unwind facts exactly as the shell does
    // after the repair: live URL is still the captured file
    // (the navigation was adopted at load start), nothing
    // committed, nothing before it, no traversal pending,
    // the tab was adopted, and it is not the only tab.
    let committed = h.current();
    assert_eq!(
        unwind_action(
            FILE,
            committed,
            None,
            FILE,
            &test_chain(),
            h.has_pending(),
            UnwindOrigin::Adopted,
            false
        ),
        UnwindAction::CloseTab
    );
}

#[test]
fn baseline_without_repair_a_committed_file_unwinds_to_start_page() {
    // The guard-free baseline that documents WHY the repair
    // matters: WITHOUT drop_top_entry, a file that actually
    // COMMITTED (record_commit, pollution arm 1) stays the
    // history's current entry. Composing the same unwind
    // facts from that polluted state — committed = file,
    // prev = None — an adopted, not-only tab would still
    // close here, but a USER tab (and the only-tab case)
    // lands on StartPage with the file left in its history;
    // worse, the polluted committed=file / origin re-derived
    // as User is exactly the WS1 shape that converted the
    // user's adopted tab to the start page. This test pins
    // the unrepaired outcome so the repair tests above have
    // a contrast.
    let mut h = TabHistory::new();
    h.record_commit(FILE);
    assert_eq!(h.current(), Some(FILE));
    assert_eq!(
        unwind_action(
            FILE,
            h.current(),
            None,
            FILE,
            &test_chain(),
            h.has_pending(),
            UnwindOrigin::User,
            false
        ),
        UnwindAction::StartPage,
        "unrepaired: the committed file entry forces StartPage"
    );
}
