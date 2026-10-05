//! Native glass under the shell's panes.
//!
//! The window is transparent and the webview draws no background, so the
//! page can leave a pane translucent and let a native material view show
//! through it. The shell reports where its glass panes are; on macOS one
//! material view goes under each (Liquid Glass on macOS 26, the HUD
//! vibrancy material before it). Elsewhere there is no native glass and the
//! shell keeps painting its own pane fills.

use serde::Deserialize;
use tauri::{Runtime, WebviewWindow};

/// A pane's rect in CSS pixels from the webview's top-left, and its corner
/// radius.
#[derive(Debug, Clone, Copy, Deserialize)]
pub(crate) struct GlassRegion {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub radius: f64,
}

/// More than the shell ever shows at once (three panes).
const MAX_REGIONS: usize = 16;
/// Larger than any display; rejects garbage without limiting real windows.
const MAX_EXTENT: f64 = 100_000.0;
const MAX_RADIUS: f64 = 256.0;

/// Validate the regions and lay them out as the native bridge reads them:
/// five values per region.
pub(crate) fn flatten(regions: &[GlassRegion]) -> Result<Vec<f64>, String> {
    if regions.len() > MAX_REGIONS {
        return Err("glass-regions-too-many".to_owned());
    }
    let mut rects = Vec::with_capacity(regions.len() * 5);
    for region in regions {
        let values = [
            region.x,
            region.y,
            region.width,
            region.height,
            region.radius,
        ];
        let in_range = values.iter().all(|value| value.is_finite())
            && region.x.abs() <= MAX_EXTENT
            && region.y.abs() <= MAX_EXTENT
            && region.width > 0.0
            && region.width <= MAX_EXTENT
            && region.height > 0.0
            && region.height <= MAX_EXTENT
            && (0.0..=MAX_RADIUS).contains(&region.radius);
        if !in_range {
            return Err("glass-region-invalid".to_owned());
        }
        rects.extend_from_slice(&values);
    }
    Ok(rects)
}

/// Put native glass under the given regions of this window, replacing any
/// regions set before. Returns whether native glass is available here; when
/// it is not, the shell keeps its own pane fills.
#[tauri::command]
pub(crate) fn set_glass_regions<R: Runtime>(
    window: WebviewWindow<R>,
    regions: Vec<GlassRegion>,
) -> Result<bool, String> {
    let rects = flatten(&regions)?;
    #[cfg(target_os = "macos")]
    {
        let ns_window = window
            .ns_window()
            .map_err(|_| "glass-window-unavailable".to_owned())? as usize;
        window
            .run_on_main_thread(move || {
                crate::native::set_glass_regions(ns_window as *mut std::ffi::c_void, &rects);
            })
            .map_err(|_| "glass-window-unavailable".to_owned())?;
        Ok(true)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (window, rects);
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn region(x: f64, y: f64, width: f64, height: f64, radius: f64) -> GlassRegion {
        GlassRegion {
            x,
            y,
            width,
            height,
            radius,
        }
    }

    #[test]
    fn regions_flatten_in_order_five_values_each() {
        let rects = flatten(&[
            region(0.0, 0.0, 400.0, 760.0, 16.0),
            region(1010.0, 0.0, 300.0, 760.0, 16.0),
        ])
        .expect("valid regions");
        assert_eq!(
            rects,
            vec![
                0.0, 0.0, 400.0, 760.0, 16.0, 1010.0, 0.0, 300.0, 760.0, 16.0
            ]
        );
    }

    #[test]
    fn no_regions_clears_the_glass() {
        assert_eq!(flatten(&[]).expect("empty is valid"), Vec::<f64>::new());
    }

    #[test]
    fn a_degenerate_or_non_finite_region_is_refused() {
        for bad in [
            region(0.0, 0.0, 0.0, 10.0, 0.0),
            region(0.0, 0.0, 10.0, -1.0, 0.0),
            region(f64::NAN, 0.0, 10.0, 10.0, 0.0),
            region(0.0, 0.0, 10.0, 10.0, f64::INFINITY),
            region(0.0, 0.0, 10.0, 10.0, -2.0),
            region(0.0, 0.0, MAX_EXTENT * 2.0, 10.0, 0.0),
        ] {
            assert_eq!(flatten(&[bad]), Err("glass-region-invalid".to_owned()));
        }
    }

    #[test]
    fn too_many_regions_are_refused() {
        let many = vec![region(0.0, 0.0, 10.0, 10.0, 0.0); MAX_REGIONS + 1];
        assert_eq!(flatten(&many), Err("glass-regions-too-many".to_owned()));
    }
}
