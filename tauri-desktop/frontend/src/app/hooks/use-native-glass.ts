import { isTauri } from "@tauri-apps/api/core";
import { useEffect } from "react";
import { type GlassRegion, setGlassRegions } from "../../lib/tauri/platform-api";

const NATIVE_GLASS_CLASS = "tc-native-glass";

function round(value: number) {
  return Math.round(value * 2) / 2;
}

/**
 * Every element marked `data-glass`, in paint order: panes first, then
 * anything inside a modal scrim, so a modal's glass stacks above the panes.
 */
function measureRegions(): GlassRegion[] {
  const elements = Array.from(
    document.querySelectorAll<HTMLElement>("[data-glass]"),
  );
  const inScrim = (element: HTMLElement) =>
    element.closest(".tc-scrim") !== null ? 1 : 0;
  return elements
    .sort((a, b) => inScrim(a) - inScrim(b))
    .map((element) => {
      const rect = element.getBoundingClientRect();
      const radius = Number.parseFloat(
        getComputedStyle(element).borderTopLeftRadius,
      );
      return {
        x: round(rect.left),
        y: round(rect.top),
        width: round(rect.width),
        height: round(rect.height),
        radius: Number.isFinite(radius) ? radius : 0,
      };
    })
    .filter((region) => region.width > 0 && region.height > 0);
}

/**
 * Keep the app's native glass under the glass panes. The window is
 * transparent; on macOS a native material view sits under each pane and
 * modal, and the page drops its own pane fills (`tc-native-glass` on
 * <html>) so the material shows through. Where there is no native glass,
 * nothing changes. Regions are re-sent only when they move or resize.
 */
export function useNativeGlass() {
  useEffect(() => {
    if (!isTauri()) return;
    const root = document.documentElement;
    let frame = 0;
    let sent = "";
    let active = true;

    const measure = () => {
      frame = 0;
      const regions = measureRegions();
      const key = JSON.stringify(regions);
      if (key === sent) return;
      sent = key;
      setGlassRegions(regions)
        .then((native) => {
          if (active) root.classList.toggle(NATIVE_GLASS_CLASS, native);
        })
        .catch(() => {
          // Keep the painted fills; try again on the next change.
          sent = "";
          root.classList.remove(NATIVE_GLASS_CLASS);
        });
    };
    const schedule = () => {
      if (frame === 0) frame = requestAnimationFrame(measure);
    };

    const sizes = new ResizeObserver(schedule);
    const observeGlass = () => {
      sizes.disconnect();
      for (const element of document.querySelectorAll("[data-glass]"))
        sizes.observe(element);
      schedule();
    };
    // Panes and modals come and go with the view; a pane can also move
    // without resizing when a neighbour opens or closes.
    const layout = new MutationObserver(observeGlass);
    layout.observe(document.body, {
      childList: true,
      subtree: true,
      attributes: true,
      attributeFilter: ["class", "style"],
    });
    window.addEventListener("resize", schedule);
    observeGlass();

    return () => {
      active = false;
      if (frame !== 0) cancelAnimationFrame(frame);
      sizes.disconnect();
      layout.disconnect();
      window.removeEventListener("resize", schedule);
      root.classList.remove(NATIVE_GLASS_CLASS);
      void setGlassRegions([]).catch(() => undefined);
    };
  }, []);
}
