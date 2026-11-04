#!/bin/bash
BIN=/Users/zakimanian/code/tc-wt-claude/native-rotation-clock/target/debug/deps/trace_commons_ingest-28103a0de1969f8a
for db in la utc; do for tz in America/Los_Angeles UTC; do
 out=$(env TZ=$tz TRACE_COMMONS_PG_TEST_DATABASE_URL=postgres://$USER@127.0.0.1:5432/admission_test_tzfinal_$db TRACE_COMMONS_LOGIN_RESOLVER_DATABASE_URL=postgres://tc_login_resolver_login@127.0.0.1:5432/admission_test_tzfinal_$db $BIN ingest::app::tests::native_session_sliding_expiry_tests:: --test-threads=1 2>&1)
 echo "db=$db TZ=$tz :: $(echo "$out" | grep '^test result')"
 echo "$out" | grep -E "absolute_cap|ignores_the_database" 
done; done
