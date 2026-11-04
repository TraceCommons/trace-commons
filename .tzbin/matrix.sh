#!/bin/bash
W=/Users/zakimanian/code/tc-wt-claude/native-rotation-clock/.tzbin
P=ingest::app::tests::native_session_sliding_expiry_tests::
for bin in before after; do
 for t in pg_native_rotation_stops_at_the_absolute_cap pg_native_rotation_cap_ignores_the_database_time_zone; do
  for db in la utc; do
   for tz in unset UTC; do
    url=postgres://$USER@127.0.0.1:5432/admission_test_tzrepro_$db
    rurl=postgres://tc_login_resolver_login@127.0.0.1:5432/admission_test_tzrepro_$db
    if [ $tz = unset ]; then envp="env -u TZ"; else envp="env TZ=UTC"; fi
    out=$($envp TRACE_COMMONS_PG_TEST_DATABASE_URL=$url TRACE_COMMONS_LOGIN_RESOLVER_DATABASE_URL=$rurl $W/$bin --exact $P$t --test-threads=1 2>&1)
    echo "$bin | $t | db=$db | TZ=$tz | $(echo "$out" | grep -E '^test result' | cut -c1-45)"
    echo "$out" | grep -E "left:|right:|panicked at" -A0 | head -4 | sed 's/^/    /'
   done
  done
 done
done
