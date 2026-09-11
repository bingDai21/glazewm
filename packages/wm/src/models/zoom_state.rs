use uuid::Uuid;

/// Zoom state of a workspace.
///
/// A workspace is considered zoomed when one of its windows has been
/// expanded to fill the entire workspace by minimizing all other windows.
#[derive(Clone, Debug)]
pub struct ZoomState {
  /// ID of the window that was zoomed.
  pub zoomed_window_id: Uuid,

  /// IDs of the windows that were minimized by the zoom, in tree order.
  pub hidden_window_ids: Vec<Uuid>,
}
