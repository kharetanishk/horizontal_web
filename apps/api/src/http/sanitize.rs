/// Replaces U+0000, which Postgres `text` rejects, with U+FFFD. Apply to every
/// user-supplied string before it reaches the database.
pub fn sanitize(s: String) -> String {
    if s.contains('\0') {
        s.replace('\0', "\u{FFFD}")
    } else {
        s
    }
}
