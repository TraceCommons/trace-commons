#!/usr/bin/env bash
#
# Runs inside `dbus-run-session` (see weston-portal-verify.sh, which is the
# only caller). Not meant to be run standalone: it assumes
# $DBUS_SESSION_BUS_ADDRESS is already a private session bus.
#
# Args: <scratch-dir> <shell-binary> <state-dir>
set -uo pipefail

WORKDIR="$1"
SHELL_BIN="$2"
TC_DIR="$3"

PORTAL_BIN=/usr/libexec/xdg-desktop-portal
PORTAL_GTK_BIN=/usr/libexec/xdg-desktop-portal-gtk

FAIL=0
fail() {
  echo "FAIL: $1" >&2
  FAIL=1
}

# The rendering fixture explicitly selects Insights and its Overview tab;
# the connected shell's default destination is Home.
#
# The popover notices are laid over the top of the tab, as on the Mac
# (owner decision 1 of the 2026-11-16 spec, chosen again 2026-11-17), and a
# build from source outside a flatpak always shows the update notice. That
# card lies over the Insights page's tab switcher (Overview, Benchmarks,
# Breakdown), which is the first thing on the page, so no switcher word is
# readable in this frame and none is asked for. The switcher's labels and
# its switching are asserted by the mounted
# `ui::insights::tests::mounted_insights_tabs_have_visible_labels_and_switch_pages`
# run above, and the notices' placement and X by
# `ui::monitor::shell::tests::mounted_notices_are_laid_over_the_tab_they_belong_to`
# and `mounted_closing_every_notice_uncovers_the_tabs_first_heading`.
#
# What the frame does require is the Overview's body, below the card: the
# "Cache share" and "Tokens by day" card headings, which only the Overview
# draws (the tab strip and the map say neither). No notice's text is asked
# for.
#
# $1 is a sparse-text OCR pass (`--psm 11`), which reads the page's text
# word by word; the capture loop below retries while the body loads.
insights_frame() {
  grep -qi 'Cache share' <<< "$1" &&
    grep -qi 'Tokens by day' <<< "$1"
}

# `cargo test <filter> -- --exact --ignored` exits 0 when the filter matches
# zero tests, which is exactly what happens if the named test is renamed or
# deleted -- a gate built on the exit code alone stays green while testing
# nothing. Capture the output and require an explicit "N passed" with N >= 1
# on top of a zero exit code.
run_ignored_test() {
  local description="$1"
  shift
  local output
  output=$("$@" 2>&1)
  local rc=$?
  echo "$output"
  if [ "$rc" -ne 0 ]; then
    fail "$description (test exited non-zero)"
    return
  fi
  if ! grep -qE 'test result: ok\. [1-9][0-9]* passed' <<< "$output"; then
    fail "$description (filter matched zero tests; cargo test exits 0 for an empty match)"
  fi
}

# --- XDG_RUNTIME_DIR: weston's socket and the portal both want one --------

if [ -z "${XDG_RUNTIME_DIR:-}" ] || [ ! -d "$XDG_RUNTIME_DIR" ]; then
  export XDG_RUNTIME_DIR="$WORKDIR/xdg-runtime"
  mkdir -p "$XDG_RUNTIME_DIR"
  chmod 700 "$XDG_RUNTIME_DIR"
fi

PIDS=()
cleanup() {
  for pid in "${PIDS[@]:-}"; do
    kill "$pid" 2>/dev/null || true
  done
}
trap cleanup EXIT

# --- axis 1: real rendering under a real compositor ------------------------

WAYLAND_SOCKET=wayland-ci
# A SOFTWARE renderer, explicitly. Without this weston finds no GL on a CI
# runner (no GPU; MESA reports "ZINK: vkCreateInstance failed" and "failed to
# get driver name for fd -1") and silently falls back to its NO-OP renderer.
# The no-op renderer composites nothing, so there is no framebuffer to capture
# and weston-screenshooter aborts on `assertion 'width > 0' failed` -- which
# reads like a screenshot tool bug and is actually "nothing was ever drawn".
#
# The flag was renamed across weston versions, so probe rather than guess:
# --renderer=pixman on weston 12+, --use-pixman before that.
if weston --help 2>&1 | grep -q -- '--renderer='; then
  RENDERER_FLAG=(--renderer=pixman)
elif weston --help 2>&1 | grep -q -- '--use-pixman'; then
  RENDERER_FLAG=(--use-pixman)
else
  echo "note: this weston exposes neither --renderer= nor --use-pixman;" >&2
  echo "      it will pick its own renderer and may fall back to no-op." >&2
  RENDERER_FLAG=()
fi

# --debug is required, not optional, on this weston: verified against
# weston 13.0.0 (the version apt installs on ubuntu-latest/24.04) that
# weston-screenshooter's capture request is refused with "unauthorized"
# without it. weston's man page states plainly why: the output-capture
# interface weston-screenshooter binds to is gated behind --debug, because
# an unrestricted client could otherwise silently read every output's
# pixels. That is a real production concern and exactly why this stays off
# by default -- but this compositor is a throwaway, single-purpose CI
# instance with nothing sensitive on it, torn down at the end of this
# script, so the tradeoff --debug makes (screenshot access for any client)
# costs nothing here.
weston --backend=headless-backend.so --width=1280 --height=900 \
  "${RENDERER_FLAG[@]}" \
  --socket="$WAYLAND_SOCKET" --idle-time=0 --debug &
WESTON_PID=$!
PIDS+=("$WESTON_PID")

WESTON_UP=0
for _ in $(seq 1 40); do
  [ -S "$XDG_RUNTIME_DIR/$WAYLAND_SOCKET" ] && { WESTON_UP=1; break; }
  sleep 0.25
done

if [ "$WESTON_UP" -ne 1 ]; then
  fail "weston headless backend never created its Wayland socket"
else
  echo "weston headless compositor is up on $WAYLAND_SOCKET"

  # Drive the real roots controls while settings publication is blocked,
  # then refused, then retried. The normal test step has no display.
  GTK_MANIFEST="$(dirname "$SHELL_BIN")/../../Cargo.toml"
  if ! WAYLAND_DISPLAY="$WAYLAND_SOCKET" GDK_BACKEND=wayland GSETTINGS_BACKEND=memory \
      cargo test --locked --manifest-path "$GTK_MANIFEST" --lib \
      ui::roots::submission_tests::pending_submission_is_single_and_failure_allows_retry \
      -- --exact --ignored --test-threads=1; then
    fail "roots submission did not preserve pending, failure and retry behavior"
  fi
  if ! WAYLAND_DISPLAY="$WAYLAND_SOCKET" GDK_BACKEND=wayland GSETTINGS_BACKEND=memory \
      cargo test --locked --manifest-path "$GTK_MANIFEST" --bin trace-commons-shell \
      startup_tests::quit_during_pending_start_suppresses_completion_and_releases_daemon \
      -- --exact --ignored --test-threads=1; then
    fail "application shutdown did not retire pending startup"
  fi

  # Insights must work before contributor state exists, and its bounded
  # worker must not publish after a window is hidden or closed.
  if ! WAYLAND_DISPLAY="$WAYLAND_SOCKET" GDK_BACKEND=wayland GSETTINGS_BACKEND=memory \
      cargo test --locked --manifest-path "$GTK_MANIFEST" --lib \
      ui::insights::tests::account_free_view_analyzes_saves_explains_deletes_and_ignores_closed_results \
      -- --exact --ignored --test-threads=1; then
    fail "local Insights lifecycle or close cancellation failed"
  fi
  if ! WAYLAND_DISPLAY="$WAYLAND_SOCKET" GDK_BACKEND=wayland GSETTINGS_BACKEND=memory \
      cargo test --locked --manifest-path "$GTK_MANIFEST" --bin trace-commons-shell \
      insights_startup_tests::first_run_local_window_does_not_create_contributor_state \
      -- --exact --ignored --test-threads=1; then
    fail "first-run Insights created contributor state"
  fi

  # Offscreen renders a mounted test writes for review, when the job asks.
  if [ -n "${TC_RENDER_DIR:-}" ]; then
    mkdir -p "$TC_RENDER_DIR"
  fi

  # Exercise the actual rooted Rust view as well as the rendered-frame OCR.
  # The background-service dialogs and the Private AI start area's "Set up"
  # (Flatpak on the host service) are mounted tests with no display
  # elsewhere in CI.
  for test_name in \
    ui::service_dialog::tests::mounted_not_installed_dialog_has_the_approved_words_and_buttons \
    ui::service_dialog::tests::mounted_not_running_dialog_has_start_it_and_the_status_command_on_failure \
    ui::service_dialog::tests::mounted_install_and_update_dialogs_show_the_status_command_after_their_last_try \
    ui::service_dialog::tests::mounted_too_old_dialog_has_the_install_command_and_copy \
    ui::service_dialog::tests::mounted_copy_command_puts_the_fixed_command_on_the_clipboard \
    ui::service_dialog::tests::mounted_commands_wrap_without_a_drawn_hyphen \
    ui::service_dialog::tests::mounted_a_narrow_command_fits_and_wraps_only_between_runs \
    ui::managed_sessions::mounted_tests::mounted_sandbox_start_area_shows_unavailable_and_set_up \
    ui::home::tests::home_links_and_failed_reads_preserve_unknowns \
    ui::home::tests::the_heroes_show_and_close_as_the_macs_do \
    ui::hero_card::tests::a_hero_is_one_button_that_hovers_and_closes_on_its_own \
    ui::hero_card::tests::mounted_the_graphic_ticks_only_while_shown_and_motion_is_wanted \
    ui::insights::tests::mounted_insights_tabs_have_visible_labels_and_switch_pages \
    ui::insights::analytics::tests::summary_actions_wrap_and_remain_keyboard_reachable \
    ui::style::tests::both_schemes_parse_with_insights_styles \
    ui::style::tests::every_omarchy_sheet_parses \
    ui::settings::navigation::tests::gtk_section_choices_scroll_and_resize_preserves_fields \
    ui::monitor::map::tests::mounted_map_preserves_identity_and_accessibility \
    ui::monitor::map::tests::mounted_map_focuses_the_camera_on_the_binoculars_tool \
    ui::monitor::map::tests::mounted_map_shows_transfer_cards_and_stops_under_reduced_motion \
    ui::monitor::map::tests::mounted_the_pile_opens_in_stages_and_the_style_switches_at_once \
    ui::monitor::map::tests::mounted_the_legend_fits_one_line_at_the_narrow_map_width \
    ui::settings::tests::mounted_the_map_style_card_has_the_shared_words \
    ui::monitor::three_pane::tests::mounted_panes_take_the_layouts_widths_and_a_hidden_pane_takes_none \
    ui::monitor::three_pane::tests::mounted_minimum_width_is_the_sum_of_the_shown_panes \
    ui::monitor::three_pane::tests::mounted_kept_widths_open_the_window_and_never_go_under_a_panes_floor \
    ui::monitor::graph_footer::tests::mounted_footer_steps_zooms_and_names_every_pill \
    ui::monitor::graph_footer::tests::mounted_footer_opens_at_once_under_reduced_motion \
    ui::monitor::graph_footer::tests::mounted_footer_controls_are_the_macs_sizes \
    ui::monitor::shell::tests::mounted_tab_strip_is_the_macs_rule_tabs \
    ui::monitor::shell::tests::mounted_toolbar_controls_are_the_macs_size \
    ui::monitor::shell::tests::mounted_high_contrast_takes_no_layout_space \
    ui::monitor::shell::tests::mounted_weston_offers_no_blur_so_the_window_stays_painted \
    ui::monitor::shell::tests::mounted_translucent_sheet_clears_the_gaps_and_tints_the_panes \
    ui::style::tests::gtk_resolves_a_redefined_colour_to_its_last_definition \
    ui::monitor::summary::tests::mounted_summary_is_the_macs_cards \
    ui::monitor::shell::tests::mounted_window_has_a_window_handle_the_desktops_controls_and_no_header_bar \
    ui::monitor::shell::tests::mounted_strip_is_the_four_tabs_and_narrows_before_onboarding \
    ui::monitor::shell::tests::mounted_tabs_are_named_by_their_titles \
    ui::monitor::shell::tests::mounted_pages_sit_behind_the_tabs_and_the_breadcrumb \
    ui::monitor::shell::tests::mounted_toggles_and_panes_follow_what_each_tab_allows \
    ui::monitor::shell::tests::mounted_notices_are_laid_over_the_tab_they_belong_to \
    ui::monitor::shell::tests::mounted_closing_every_notice_uncovers_the_tabs_first_heading \
    ui::monitor::shell::tests::mounted_a_notices_x_closes_that_card \
    ui::monitor::shell::tests::mounted_a_new_card_slides_in_and_a_revealed_one_fades \
    ui::monitor::shell::tests::mounted_the_main_pane_never_scrolls_sideways \
    ui::monitor::shell::tests::mounted_a_hidden_graph_leaves_the_tab_the_whole_pane \
    ui::monitor::shell::tests::mounted_a_front_cards_buttons_are_whole_at_every_pane_width \
    ui::monitor::shell::tests::mounted_a_covered_heading_shows_nothing_beside_the_card \
    ui::wrap_row::tests::mounted_wrapped_buttons_stay_inside_their_card \
    ui::tool_tile::mounted::mounted_a_tile_is_the_macs_size_and_says_nothing \
    ui::armed_mark::tests::mounted_the_mark_turns_to_a_check_with_a_pulse \
    ui::armed_mark::tests::mounted_the_armed_card_goes_once \
    ui::glass_feedback::tests::mounted_a_pressed_refresh_turns_once \
    ui::folder_handoff::tests::mounted_the_folder_slides_in_once \
    ui::insights::tests::mounted_overview_fits_the_main_pane_with_equal_insets \
    ui::monitor::traces_list::mounted_tests::mounted_the_folder_list_is_the_macs \
    ui::monitor::traces_list::mounted_tests::mounted_the_folder_lists_icons_are_in_the_theme \
    ui::monitor::traces_list::mounted_tests::mounted_a_selected_rows_submit_is_the_macs_control_on_the_selection \
    ui::monitor::traces_list::mounted_tests::mounted_the_row_menus_dismiss_asks_first \
    ui::monitor::shell::fit_tests::every_hosted_screen_fits_the_440_main_pane \
    ui::queue::consent_mounted_tests::mounted_folder_undo_cancels_by_project_and_waits_for_the_daemon \
    ui::queue::consent_mounted_tests::mounted_trace_undo_cancels_by_entry_id \
    ui::queue::consent_mounted_tests::mounted_held_trace_shows_its_reasons_and_no_row_submit \
    ui::settings::consent_mounted_tests::mounted_settings_have_no_undo_window_control \
    ui::settings::consent_mounted_tests::mounted_sharing_ask_each_time_withdraws_the_grant \
    ui::settings::consent_mounted_tests::mounted_sharing_automatic_asks_about_ask_folders_first \
    ui::settings::consent_mounted_tests::mounted_sharing_automatic_refuses_on_an_unreadable_configuration \
    ui::settings::consent_mounted_tests::mounted_uses_sends_the_daemons_list_with_one_scope_changed \
    ui::terms_hold::consent_mounted_tests::mounted_terms_hold_notice_follows_the_hold \
    ui::history::consent_mounted_tests::mounted_previous_accounts_row_offers_no_withdraw \
    ui::history::consent_mounted_tests::mounted_an_opened_record_shows_its_contribution_details \
    ui::history::consent_mounted_tests::mounted_withdraw_asks_first_then_sends_the_record \
    ui::history::consent_mounted_tests::mounted_tab_reaches_a_history_folders_records \
    ui::style::tests::mounted_tab_passes_a_grid_of_figures \
    ui::queue::consent_mounted_tests::mounted_an_undo_reply_clears_only_the_undo_that_was_pressed \
    ui::queue::consent_mounted_tests::mounted_not_this_one_asks_first \
    ui::queue::consent_mounted_tests::mounted_preview_sheet_says_why_held_and_sends_nothing \
    ui::queue::consent_mounted_tests::mounted_the_sheet_buttons_end_inside_the_sheet \
    ui::settings::consent_mounted_tests::mounted_an_override_shows_on_the_sharing_card_with_no_clear \
    ui::settings::consent_mounted_tests::mounted_bulk_change_plans_asks_and_sends_what_was_agreed \
    ui::settings::consent_mounted_tests::mounted_turning_on_training_offers_it_on_sent_traces \
    ui::first_run::dark_tests::welcome_is_readable_in_dark_and_title_is_larger_than_body \
    ui::first_run::dark_tests::the_scoped_stage_leaves_the_main_window_light \
    ui::first_run_screens::mounted_tests::welcome_offers_get_started_and_the_agent_prompt \
    ui::first_run_screens::mounted_tests::tools_start_unticked_and_continue_waits_for_a_watch \
    ui::first_run_screens::mounted_tests::tools_tick_a_tool_the_daemon_already_watches \
    ui::first_run_screens::mounted_tests::a_folder_the_old_portal_cannot_map_shows_the_line_and_a_copy_only_command \
    ui::first_run_screens::mounted_tests::a_refusal_hides_once_the_daemon_holds_the_refused_tool \
    ui::first_run_screens::mounted_tests::a_refusal_for_a_held_tool_hides_once_its_declaration_is_written \
    ui::first_run_screens::mounted_tests::private_ai_counts_the_tools_that_can_connect \
    ui::first_run_screens::mounted_tests::the_uses_flow_names_its_gate_and_the_commons \
    ui::first_run_terms::mounted_tests::terms_sits_in_the_same_frame_as_the_other_steps \
    ui::first_run_screens::mounted_tests::the_uses_captions_sit_under_their_nodes \
    ui::first_run_screens::mounted_tests::the_marker_check_ticks_only_while_its_stroke_moves \
    ui::first_run_screens::mounted_tests::the_customize_card_rises_to_its_place \
    ui::first_run_stage_view::mounted_tests::the_stage_draws_under_the_steps_and_ticks_only_while_shown \
    ui::first_run_stage_view::mounted_tests::a_tool_watched_with_animations_off_is_drawn_lit \
    ui::first_run_stage_view::mounted_tests::welcome_lines_rise_and_replay_after_the_agent_prompt \
    ui::first_run_stage_view::mounted_tests::the_threads_way_back_follows_a_commit; do
    WAYLAND_DISPLAY="$WAYLAND_SOCKET" GDK_BACKEND=wayland GSETTINGS_BACKEND=memory \
      run_ignored_test "GTK display regression failed: $test_name" \
      cargo test --locked --manifest-path "$GTK_MANIFEST" --lib "$test_name" \
      -- --exact --ignored --test-threads=1
  done

  # Mission drafts are absent while missions are hidden and, with the switch
  # on, reachable without starting Contributions, and
  # inspection/quit failures must preserve the live local view.
  WAYLAND_DISPLAY="$WAYLAND_SOCKET" GDK_BACKEND=wayland GSETTINGS_BACKEND=memory \
    run_ignored_test "local-first mission draft navigation required contributor startup" \
    cargo test --locked --manifest-path "$GTK_MANIFEST" --lib \
    ui::insights::tests::local_first_window_lists_mission_drafts_only_while_missions_are_shown \
    -- --exact --ignored --test-threads=1
  WAYLAND_DISPLAY="$WAYLAND_SOCKET" GDK_BACKEND=wayland GSETTINGS_BACKEND=memory \
    run_ignored_test "mission draft lifecycle, failed inspection or declined close failed" \
    cargo test --locked --manifest-path "$GTK_MANIFEST" --lib \
    ui::mission_drafts::tests::account_free_view_imports_shows_and_deletes_plain_text_draft \
    -- --exact --ignored --test-threads=1

  # --- axis 2: a real portal daemon ------------------------------------------
  if ! WAYLAND_DISPLAY="$WAYLAND_SOCKET" GDK_BACKEND=wayland GSETTINGS_BACKEND=memory \
      cargo test --locked --manifest-path "$GTK_MANIFEST" --lib \
      ui::credential::provider_tests::settings_sign_in_signals_preserve_pending_and_recover \
      -- --exact --ignored --test-threads=1; then
    fail "Settings sign-in controls did not preserve pending, failure and retry behavior"
  fi
  if ! WAYLAND_DISPLAY="$WAYLAND_SOCKET" GDK_BACKEND=wayland GSETTINGS_BACKEND=memory \
      cargo test --locked --manifest-path "$GTK_MANIFEST" --lib \
      ui::funding::widget_tests::billing_widgets_bind_and_invalidate_browser_handoffs \
      -- --exact --ignored --test-threads=1; then
    fail "billing controls did not preserve organization binding and invalidation"
  fi

  #
  # ORDER MATTERS, and getting it wrong is why the first run of this job proved
  # nothing about the portal. xdg-desktop-portal-gtk is itself a GTK application
  # and needs a display to start. Launched before the compositor exists it dies
  # immediately, and the log fills with
  #   Activated service 'org.freedesktop.impl.portal.desktop.gtk' failed:
  #   Process ... exited with status 1
  # repeated once per portal interface -- while the FRONTEND still claims
  # org.freedesktop.portal.Desktop perfectly happily. So a bus-name check alone
  # reports a healthy portal with no backend behind it at all.
  #
  # Weston is therefore started above, and the backend is given WAYLAND_DISPLAY.

  WAYLAND_DISPLAY="$WAYLAND_SOCKET" "$PORTAL_BIN" &
  PIDS+=("$!")
  WAYLAND_DISPLAY="$WAYLAND_SOCKET" GDK_BACKEND=wayland "$PORTAL_GTK_BIN" &
  PIDS+=("$!")

  # The frontend owning the bus name is NOT evidence the backend is alive -- see
  # above. Assert the backend separately.
  BACKEND_UP=0
  for _ in $(seq 1 20); do
    if gdbus call --session --dest org.freedesktop.DBus \
        --object-path /org/freedesktop/DBus \
        --method org.freedesktop.DBus.NameHasOwner \
        org.freedesktop.impl.portal.desktop.gtk 2>/dev/null | grep -q 'true'; then
      BACKEND_UP=1
      break
    fi
    sleep 0.5
  done
  if [ "$BACKEND_UP" -ne 1 ]; then
    fail "the gtk portal BACKEND never came up (org.freedesktop.impl.portal.desktop.gtk has no owner) -- a frontend-only portal is not the thing this job claims to verify"
  else
    echo "gtk portal backend is alive and owns org.freedesktop.impl.portal.desktop.gtk"
  fi

  (
    cd "$WORKDIR" || exit 1
    WAYLAND_DISPLAY="$WAYLAND_SOCKET" GDK_BACKEND=wayland GSETTINGS_BACKEND=memory \
      GSK_RENDERER=cairo "$SHELL_BIN" --state-dir "$TC_DIR" --start-page insights --exit-after-realize --realize-seconds 60 \
      >"$WORKDIR/app.log" 2>&1 &
    APP_PID=$!

    # Use GTK's software renderer on a compositor with no GPU, and keep
    # the app alive beyond all ten capture attempts. A ten-second lifetime
    # could end before text was captured and leave only desktop screenshots.
    # Give the window time to realize and composite at least one frame
    # before asking the compositor for a screenshot.
    # Capture until the frame actually contains text, not once after a fixed
    # wait.
    #
    # `sleep 6` then one shot was a race, and it failed in the way races do:
    # the job went red on documentation PRs that never touch this crate, with
    # "OCR did not find the expected header-bar text". Comparing artifacts
    # showed a 76 KB frame on the failures against 158 KB on the pass -- the
    # window had been composited enough to be non-uniform (so the blank check
    # passed) but had not finished painting its text. Under a loaded runner
    # six seconds is simply not a guarantee.
    #
    # This does NOT weaken the assertion: the loop still requires readable
    # text, and still fails if it never appears. It removes a timing
    # dependency, which is the thing that was making a required gate flaky and
    # therefore ignorable.
    CAPTURED=0
    for attempt in $(seq 1 10); do
      sleep 3
      rm -f "$WORKDIR"/*.png 2>/dev/null || true
      WAYLAND_DISPLAY="$WAYLAND_SOCKET" weston-screenshooter || true
      CANDIDATE=$(ls -t "$WORKDIR"/*.png 2>/dev/null | head -1 || true)
      [ -z "$CANDIDATE" ] && continue
      CANDIDATE_TEXT=$(tesseract "$CANDIDATE" - --psm 11 2>/dev/null || true)
      if insights_frame "$CANDIDATE_TEXT"; then
        echo "frame with readable text captured on attempt $attempt"
        CAPTURED=1
        break
      fi
      echo "attempt $attempt: frame captured but no readable text yet"
    done
    if [ "$CAPTURED" -ne 1 ]; then
      echo "no frame containing readable text after 10 attempts" >&2
    fi

    wait "$APP_PID"
  )

  echo "--- application log ---"
  cat "$WORKDIR/app.log" 2>/dev/null || true
  if grep -q "panicked at" "$WORKDIR/app.log"; then
    fail "application background thread panicked during desktop smoke"
  fi
  echo "-----------------------"

  SHOT=$(ls -t "$WORKDIR"/*.png 2>/dev/null | head -1 || true)
  if [ -z "$SHOT" ]; then
    fail "weston-screenshooter produced no PNG -- cannot check rendering"
  else
    echo "screenshot: $SHOT ($(stat -c%s "$SHOT" 2>/dev/null || echo '?') bytes)"
    # CI uploads the frame the checks below read, so a failed OCR can be
    # looked at rather than guessed at from the text alone.
    if [ -n "${WESTON_SHOT_OUT:-}" ]; then
      cp "$SHOT" "$WESTON_SHOT_OUT" || true
    fi

    # Not blank/uniform: a flat-colour frame (nothing composited, or a
    # black/white rectangle) has a standard deviation at or near zero. This
    # is deliberately a low bar -- it is a floor under "the compositor
    # actually received pixels from the app", not a claim about layout
    # quality.
    STDDEV=$(convert "$SHOT" -colorspace Gray -format "%[fx:standard_deviation]" info: 2>/dev/null || echo "0")
    echo "grayscale standard deviation: $STDDEV"
    PASSES_STDDEV=$(awk -v v="$STDDEV" 'BEGIN { print (v+0 > 0.01) ? "1" : "0" }')
    if [ "$PASSES_STDDEV" != "1" ]; then
      fail "screenshot is blank/uniform (stddev=$STDDEV) -- nothing was actually composited"
    else
      echo "screenshot is not blank/uniform"
    fi

    if command -v tesseract >/dev/null 2>&1; then
      OCR_TEXT=$(tesseract "$SHOT" stdout --psm 11 2>/dev/null || echo "")
      echo "--- OCR text (sparse, --psm 11) ---"
      echo "$OCR_TEXT"
      echo "----------------"
      if insights_frame "$OCR_TEXT"; then
        echo "OCR found the Overview's Cache share and Tokens by day cards"
      else
        fail "OCR did not find the Overview's Cache share and Tokens by day cards in the screenshot"
      fi
    else
      fail "tesseract not available -- OCR text-presence check could not run"
    fi
  fi
fi

# --- every window carries the app id -----------------------------------------
#
# A window GTK does not hold as one of the application's (the first run, the
# Look inside sheet, every dialog) takes its Wayland app id from the program
# name, which was the binary's (`trace-commons-shell`), so the desktop could
# not match it to ai.tracecommons.Contributor.desktop. The client's own
# protocol log names every toplevel's app id; each must be the app's.
if [ -n "${WAYLAND_SOCKET:-}" ]; then
  APP_IDS=$(
    cd "$WORKDIR" &&
      WAYLAND_DEBUG=1 WAYLAND_DISPLAY="$WAYLAND_SOCKET" GDK_BACKEND=wayland GSETTINGS_BACKEND=memory \
        GSK_RENDERER=cairo timeout 60 "$SHELL_BIN" --state-dir "$TC_DIR" --exit-after-realize --realize-seconds 8 2>&1 |
      grep -o 'set_app_id("[^"]*")' | sort | uniq -c
  )
  echo "toplevel app ids: ${APP_IDS:-none}"
  if [ -z "$APP_IDS" ]; then
    fail "no toplevel set an app id -- the app-id check read nothing"
  elif grep -v 'set_app_id("ai.tracecommons.Contributor")' <<< "$APP_IDS" | grep -q .; then
    fail "a window's app id is not ai.tracecommons.Contributor"
  else
    echo "every toplevel's app id is ai.tracecommons.Contributor"
  fi
fi

# --- axis 2: bounded reply from the independent live portal probe ----------

PORTAL_UP=0
for _ in $(seq 1 20); do
  if gdbus call --session \
      --dest org.freedesktop.DBus \
      --object-path /org/freedesktop/DBus \
      --method org.freedesktop.DBus.NameHasOwner \
      org.freedesktop.portal.Desktop 2>/dev/null | grep -q 'true'; then
    PORTAL_UP=1
    break
  fi
  sleep 0.5
done

if [ "$PORTAL_UP" -ne 1 ]; then
  fail "xdg-desktop-portal never claimed org.freedesktop.portal.Desktop on the session bus"
else
  echo "portal daemon is alive and owns org.freedesktop.portal.Desktop"

  # Independent of the app: call RequestBackground directly and check the
  # failure mode. A live portal with no Background backend must answer
  # (with an error) inside a bounded time, and that error must not be the
  # ServiceUnknown/NameHasNoOwner class -- that is exactly what "nothing is
  # listening at all" looks like, and getting it here would mean this
  # setup is not actually testing anything different from headless-run.sh.
  PROBE_OUT=$(timeout 10 gdbus call --session \
    --dest org.freedesktop.portal.Desktop \
    --object-path /org/freedesktop/portal/desktop \
    --method org.freedesktop.portal.Background.RequestBackground "" "{}" 2>&1)
  PROBE_RC=$?
  echo "RequestBackground probe (rc=$PROBE_RC): $PROBE_OUT"

  if [ "$PROBE_RC" -eq 124 ]; then
    fail "RequestBackground did not return within 10s against a live portal -- should fail fast with no Background backend registered, not hang"
  elif echo "$PROBE_OUT" | grep -Eqi 'ServiceUnknown|NameHasNoOwner|was not provided by any \.service files'; then
    fail "RequestBackground got the same 'nothing is listening' error as no-portal-at-all -- the live daemon above did not actually field the call"
  else
    echo "RequestBackground reached the live portal daemon and got a real (non-absence) reply"
  fi
fi

# Preserve failures from both rendering and the independent portal probe.
exit "$FAIL"
