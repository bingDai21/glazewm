use anyhow::Context;
use tracing::info;
use wm_common::WindowState;

use crate::{
  commands::window::update_window_state,
  models::{WindowContainer, Workspace, ZoomState},
  traits::{CommonGetters, WindowGetters},
  user_config::UserConfig,
  wm_state::WmState,
};

/// Toggles zoom of the workspace containing the given window.
///
/// When zooming, all other windows in the workspace are minimized so
/// that the given window fills the entire workspace. When unzooming, the
/// windows that were previously minimized by the zoom are restored to
/// their prior states and tiling positions.
#[allow(clippy::needless_pass_by_value)]
pub fn toggle_zoom(
  window: WindowContainer,
  state: &mut WmState,
  config: &UserConfig,
) -> anyhow::Result<()> {
  let workspace = window.workspace().context("No workspace.")?;

  match workspace.zoom_state() {
    Some(zoom_state) => {
      unzoom_workspace(&workspace, &zoom_state, state, config)
    }
    None => zoom_window(&window, &workspace, state, config),
  }
}

/// Zooms the given window by minimizing all other windows in its
/// workspace.
///
/// Does nothing if the workspace has no other non-minimized windows.
fn zoom_window(
  window: &WindowContainer,
  workspace: &Workspace,
  state: &mut WmState,
  config: &UserConfig,
) -> anyhow::Result<()> {
  // Get all other windows in the workspace that aren't already
  // minimized, in tree order.
  let windows_to_minimize = workspace
    .descendants()
    .filter_map(|container| container.as_window_container().ok())
    .filter(|descendant| {
      descendant.id() != window.id()
        && descendant.state() != WindowState::Minimized
    })
    .collect::<Vec<_>>();

  // No-op if there are no other windows to minimize.
  if windows_to_minimize.is_empty() {
    return Ok(());
  }

  info!("Zooming window: {window}");

  let hidden_window_ids =
    windows_to_minimize.iter().map(CommonGetters::id).collect();

  workspace.set_zoom_state(Some(ZoomState {
    zoomed_window_id: window.id(),
    hidden_window_ids,
  }));

  for window_to_minimize in windows_to_minimize {
    update_window_state(
      window_to_minimize,
      WindowState::Minimized,
      state,
      config,
    )?;
  }

  Ok(())
}

/// Unzooms the given workspace by restoring the windows that were
/// minimized by the zoom.
///
/// Windows that are no longer managed or are no longer minimized (e.g.
/// manually unminimized by the user) are skipped.
fn unzoom_workspace(
  workspace: &Workspace,
  zoom_state: &ZoomState,
  state: &mut WmState,
  config: &UserConfig,
) -> anyhow::Result<()> {
  info!("Unzooming workspace: {workspace}");

  workspace.set_zoom_state(None);

  for window_id in &zoom_state.hidden_window_ids {
    let window = state
      .container_by_id(*window_id)
      .and_then(|container| container.as_window_container().ok());

    // Skip windows that are no longer managed or are no longer
    // minimized.
    let Some(window) =
      window.filter(|window| window.state() == WindowState::Minimized)
    else {
      continue;
    };

    // Restore the window to its state prior to being minimized.
    let target_state = window
      .prev_state()
      .unwrap_or_else(|| WindowState::default_from_config(&config.value));

    update_window_state(window, target_state, state, config)?;
  }

  Ok(())
}

#[cfg(test)]
mod tests {
  use tokio::sync::mpsc;
  use wm_common::{FloatingStateConfig, WindowState};
  use wm_platform::Dispatcher;

  use super::*;
  use crate::{
    commands::container::attach_container,
    models::{Monitor, NonTilingWindow, TilingWindow},
    traits::WindowGetters,
  };

  /// Creates a `WmState` with a single monitor containing the given
  /// workspace, alongside a default `UserConfig`.
  fn state_with_workspace(
    workspace: &Workspace,
  ) -> anyhow::Result<(WmState, UserConfig)> {
    let (event_tx, _event_rx) = mpsc::unbounded_channel();
    let (exit_tx, _exit_rx) = mpsc::unbounded_channel();
    let state = WmState::new(Dispatcher::mock(), event_tx, exit_tx);

    let monitor =
      Monitor::mock().workspaces(vec![workspace.clone()]).call();
    attach_container(
      &monitor.into(),
      &state.root_container.clone().into(),
      None,
    )?;

    Ok((state, UserConfig::mock()))
  }

  #[test]
  fn zoom_minimizes_other_windows() -> anyhow::Result<()> {
    let window_a = TilingWindow::mock().call();
    let window_b = TilingWindow::mock().call();
    let workspace = Workspace::mock()
      .tiling_containers(vec![
        window_a.clone().into(),
        window_b.clone().into(),
      ])
      .call();
    let (mut state, config) = state_with_workspace(&workspace)?;

    toggle_zoom(window_a.clone().into(), &mut state, &config)?;

    let zoom_state =
      workspace.zoom_state().expect("Workspace should be zoomed.");

    assert_eq!(zoom_state.zoomed_window_id, window_a.id());
    assert_eq!(zoom_state.hidden_window_ids, vec![window_b.id()]);

    Ok(())
  }

  #[test]
  fn zoom_is_noop_without_other_windows() -> anyhow::Result<()> {
    let window = TilingWindow::mock().call();
    let workspace = Workspace::mock()
      .tiling_containers(vec![window.clone().into()])
      .call();
    let (mut state, config) = state_with_workspace(&workspace)?;

    toggle_zoom(window.into(), &mut state, &config)?;

    assert!(workspace.zoom_state().is_none());

    Ok(())
  }

  #[test]
  fn unzoom_restores_hidden_windows() -> anyhow::Result<()> {
    let window_a = TilingWindow::mock().call();
    let window_b =
      NonTilingWindow::mock().state(WindowState::Minimized).call();
    let window_c =
      NonTilingWindow::mock().state(WindowState::Minimized).call();
    window_b.set_prev_state(WindowState::Tiling);
    window_c.set_prev_state(WindowState::Floating(
      FloatingStateConfig::default(),
    ));

    let workspace = Workspace::mock()
      .tiling_containers(vec![window_a.clone().into()])
      .non_tiling_windows(vec![window_b.clone(), window_c.clone()])
      .call();
    let (mut state, config) = state_with_workspace(&workspace)?;

    workspace.set_zoom_state(Some(ZoomState {
      zoomed_window_id: window_a.id(),
      hidden_window_ids: vec![window_b.id(), window_c.id()],
    }));

    toggle_zoom(window_a.into(), &mut state, &config)?;

    assert!(workspace.zoom_state().is_none());

    // Window B should be restored to its previous tiling state.
    let restored_b = state
      .container_by_id(window_b.id())
      .expect("Window B should still be managed.");
    assert!(restored_b.is_tiling_window());

    // Window C should be restored to its previous floating state.
    let restored_c = state
      .container_by_id(window_c.id())
      .and_then(|container| container.as_window_container().ok())
      .expect("Window C should still be managed.");
    assert!(matches!(restored_c.state(), WindowState::Floating(_)));

    Ok(())
  }

  #[test]
  fn unzoom_skips_windows_no_longer_minimized() -> anyhow::Result<()> {
    let window_a = TilingWindow::mock().call();
    let window_b = TilingWindow::mock().call();
    let workspace = Workspace::mock()
      .tiling_containers(vec![
        window_a.clone().into(),
        window_b.clone().into(),
      ])
      .call();
    let (mut state, config) = state_with_workspace(&workspace)?;

    // Simulate a window that was manually unminimized while zoomed.
    workspace.set_zoom_state(Some(ZoomState {
      zoomed_window_id: window_a.id(),
      hidden_window_ids: vec![window_b.id()],
    }));

    toggle_zoom(window_a.into(), &mut state, &config)?;

    assert!(workspace.zoom_state().is_none());
    assert!(state.container_by_id(window_b.id()).is_some());

    Ok(())
  }
}
