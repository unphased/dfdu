use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crossbeam_channel::bounded;
use ratatui::layout::Rect;

use super::*;
use crate::app::Overlay;
use crate::model::snapshot::{Snapshot, test_entry};
use crate::tests::render_buffer;
use crate::theme::Theme;

const WAIT: Duration = Duration::from_secs(5);

fn image(name: &str) -> Request {
    Request::new(name.into(), Rect::new(40, 3, 20, 10))
}

#[derive(Clone, Default)]
struct Output(Arc<Mutex<Vec<u8>>>);

impl Write for Output {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Output {
    fn take(&self) -> Vec<u8> {
        std::mem::take(&mut *self.0.lock().unwrap())
    }
}

#[test]
fn decode_is_async_cached_and_written_only_after_a_frame() {
    let (started, starts) = bounded(1);
    let (finish, finishes) = bounded(1);
    let out = Output::default();
    let mut preview = Preview::with_decoder(out.clone(), move |request| {
        started.send(request.clone()).unwrap();
        finishes.recv().unwrap();
        Some(b"graphics".to_vec())
    })
    .unwrap();
    let request = image("a.jpg");
    preview.after_draw(Some(request.clone())).unwrap();
    assert_eq!(starts.recv_timeout(WAIT).unwrap(), request);
    assert!(
        out.take().is_empty(),
        "decode must not write to the terminal"
    );
    finish.send(()).unwrap();
    let decoded = preview.results().recv_timeout(WAIT).unwrap();
    preview.complete(decoded);
    assert!(out.take().is_empty(), "completion must wait for draw");
    preview.after_draw(Some(request.clone())).unwrap();
    assert_eq!(out.take(), b"graphics");
    preview.after_draw(Some(request)).unwrap();
    assert!(
        out.take().is_empty(),
        "unchanged frames must not retransmit graphics"
    );
    assert!(starts.try_recv().is_err(), "redraw must not decode again");
    preview.after_draw(None).unwrap();
    assert_eq!(out.take(), jpg_preview::delete_image(IMAGE_ID));
    drop(preview);
    assert!(out.take().is_empty(), "already cleared");
}

#[test]
fn stale_work_is_discarded_and_only_the_latest_selection_is_decoded() {
    let (started, starts) = bounded(1);
    let (finish, finishes) = bounded(1);
    let out = Output::default();
    let mut preview = Preview::with_decoder(out.clone(), move |request| {
        started.send(request.clone()).unwrap();
        finishes.recv().unwrap();
        Some(request.path.to_string_lossy().as_bytes().to_vec())
    })
    .unwrap();
    preview.after_draw(Some(image("a.jpg"))).unwrap();
    starts.recv_timeout(WAIT).unwrap();
    preview.after_draw(Some(image("b.jpg"))).unwrap();
    preview.after_draw(Some(image("c.jpg"))).unwrap();
    assert!(starts.try_recv().is_err(), "only one decode may run");
    finish.send(()).unwrap();
    let decoded = preview.results().recv_timeout(WAIT).unwrap();
    preview.complete(decoded);
    preview.after_draw(Some(image("c.jpg"))).unwrap();
    assert!(out.take().is_empty(), "stale a must never appear");
    assert_eq!(starts.recv_timeout(WAIT).unwrap(), image("c.jpg"));
    preview.clear().unwrap(); // view switch, suspend, reopen or quit
    finish.send(()).unwrap();
    let decoded = preview.results().recv_timeout(WAIT).unwrap();
    preview.complete(decoded);
    preview.after_draw(None).unwrap();
    assert!(
        out.take().is_empty(),
        "completion after cleanup must not appear"
    );
}

#[test]
fn selection_and_geometry_changes_delete_before_replacement_and_drop_cleans_up() {
    let out = Output::default();
    let mut preview = Preview::with_decoder(out.clone(), |_| Some(b"image".to_vec())).unwrap();
    let first = image("a.png");
    preview.after_draw(Some(first.clone())).unwrap();
    let decoded = preview.results().recv_timeout(WAIT).unwrap();
    preview.complete(decoded);
    preview.after_draw(Some(first.clone())).unwrap();
    assert_eq!(out.take(), b"image");
    let resized = Request::new(first.path, Rect::new(20, 4, 10, 5));
    preview.after_draw(Some(resized.clone())).unwrap();
    assert_eq!(out.take(), jpg_preview::delete_image(IMAGE_ID));
    let decoded = preview.results().recv_timeout(WAIT).unwrap();
    preview.complete(decoded);
    preview.after_draw(Some(resized)).unwrap();
    assert_eq!(out.take(), b"image");
    drop(preview);
    assert_eq!(out.take(), jpg_preview::delete_image(IMAGE_ID));
}

#[test]
fn real_worker_fits_up_and_encodes_the_fitted_placement() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("square.png");
    std::fs::write(
        &path,
        [
            137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1,
            8, 4, 0, 0, 0, 181, 28, 12, 2, 0, 0, 0, 11, 73, 68, 65, 84, 120, 218, 99, 100, 248, 15,
            0, 1, 5, 1, 1, 39, 24, 227, 102, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
        ],
    )
    .unwrap();
    let out = Output::default();
    let mut preview = Preview::new(out.clone(), true).unwrap();
    let mut request = Request::new(path, Rect::new(40, 2, 40, 20));
    request.row = 21; // last row: only the upward fit exists
    preview.after_draw(Some(request.clone())).unwrap();
    let decoded = preview.results().recv_timeout(WAIT).unwrap();
    assert!(decoded.stream.is_some());
    preview.complete(decoded);
    preview.after_draw(Some(request)).unwrap();
    let stream = String::from_utf8(out.take()).unwrap();
    assert!(stream.starts_with("\x1b7\x1b[22;41H"));
    assert!(stream.contains("c=1,r=1,"), "1x1 image stays 1 cell");
}

#[test]
fn scrollbar_column_is_not_available_to_the_preview() {
    let now = Instant::now();
    let mut app = App::new(Theme::ansi16(), now);
    app.session.outside.local = true;
    app.refresh(
        Snapshot {
            dir_path: "/r".into(),
            entries: (0..50)
                .map(|i| test_entry(i, &format!("{i}.jpg"), 100))
                .collect(),
            ..Snapshot::default()
        },
        now,
    );
    let screen = render_buffer(&mut app, now, 100, 30).area;
    let selected = request(&app, screen).unwrap();
    assert_eq!(selected.area.right(), app.areas.list.right() - 1);
    assert_eq!(selected.area.y, app.areas.list.y);
    assert_eq!(selected.area.bottom(), app.areas.list.bottom());
}

#[test]
fn disabled_preview_does_not_start_work_or_write_graphics() {
    let out = Output::default();
    let mut preview = Preview::new(out.clone(), false).unwrap();
    preview.after_draw(Some(image("a.jpg"))).unwrap();
    assert!(!preview.busy);
    assert!(preview.results().try_recv().is_err());
    drop(preview);
    assert!(out.take().is_empty());
}

#[test]
fn failed_decode_is_not_retried_on_each_frame() {
    let mut preview = Preview::with_decoder(Vec::new(), |_| None).unwrap();
    preview.after_draw(Some(image("missing.jpg"))).unwrap();
    let decoded = preview.results().recv_timeout(WAIT).unwrap();
    preview.complete(decoded);
    preview.after_draw(Some(image("missing.jpg"))).unwrap();
    assert!(!preview.busy);
    assert!(preview.results().try_recv().is_err());
}

#[test]
fn only_local_nonimported_files_in_an_unobscured_files_view_get_a_request() {
    let now = Instant::now();
    let mut app = App::new(Theme::ansi16(), now);
    app.session.outside.local = true;
    app.refresh(
        Snapshot {
            dir_path: "/r".into(),
            entries: vec![test_entry(0, "a.jpg", 100)],
            ..Snapshot::default()
        },
        now,
    );
    let screen = render_buffer(&mut app, now, 100, 30).area;
    let selected = request(&app, screen).expect("local image");
    assert_eq!(
        selected.path,
        app.session.snap.dir_path.join(b"a.jpg").to_local()
    );
    assert_eq!(selected.area.x, app.mouse.name_x + 2 + 5 + 1);
    assert_eq!(selected.area.y, app.areas.list.y);
    assert_eq!(selected.area.right(), app.areas.list.right());
    assert_eq!(selected.area.bottom(), app.areas.list.bottom());
    assert_eq!(selected.row, app.areas.list.y);
    let narrow = render_buffer(&mut app, now, 39, 10).area;
    let clipped = request(&app, narrow).expect("narrow screen still has room beside name");
    assert_eq!(clipped.area.x, app.mouse.name_x + 2 + 5 + 1);
    assert_eq!(clipped.area.width, app.areas.list.right() - clipped.area.x);
    assert_eq!(clipped.area.bottom(), app.areas.list.bottom());
    render_buffer(&mut app, now, 100, 30);
    assert_eq!(clip(selected.area, screen), Some(selected.area));
    for (name, cells) in [("界.jpg", 6), ("a-very-long-selected-image-name.jpg", 35)] {
        app.session.snap.entries[0].name = name.as_bytes().into();
        let screen = render_buffer(&mut app, now, 120, 30).area;
        let area = request(&app, screen).unwrap().area;
        assert_eq!(area.x, app.mouse.name_x + 2 + cells + 1);
        assert_eq!(area.right(), app.areas.list.right());
    }
    app.session.snap.entries[0].name = b"a.jpg".as_slice().into();
    render_buffer(&mut app, now, 100, 30);
    app.session.outside.local = false;
    assert!(request(&app, screen).is_none());
    app.session.outside.local = true;
    app.session.from_file = Some(dfdu_session::goto::FROM_FILE);
    assert!(request(&app, screen).is_none());
    app.session.from_file = None;
    app.overlay = Overlay::Roots(0);
    render_buffer(&mut app, now, 100, 30);
    assert!(request(&app, screen).is_none());
    app.overlay = Overlay::None;
    app.session.view = dfdu_session::view::View::Map;
    render_buffer(&mut app, now, 100, 30);
    assert!(request(&app, screen).is_none());
    app.session.view = dfdu_session::view::View::Files;
    app.session.snap.entries[0].is_dir = true;
    render_buffer(&mut app, now, 100, 30);
    assert!(request(&app, screen).is_none());
    app.session.snap.entries[0].is_dir = false;
    render_buffer(&mut app, now, 20, 6);
    assert!(request(&app, Rect::new(0, 0, 20, 6)).is_none());
}
