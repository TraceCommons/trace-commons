-- Qualified OpenCode 1.19.0 schema and synthetic rows, captured after importing
-- completed.json into an isolated temporary XDG_DATA_HOME on 2026-10-14.
-- Only session/message/part tables are copied. No account/credential data.
-- OpenCode import relocates session.directory to the temporary working directory;
-- original message paths, imported version 1.18.29, IDs, times and bodies are retained.
-- Foreign keys are disabled only while restoring these three isolated tables;
-- the original project table is deliberately outside this transcript fixture.
PRAGMA foreign_keys=OFF;
CREATE TABLE `session` (
          `id` text PRIMARY KEY,
          `project_id` text NOT NULL,
          `workspace_id` text,
          `parent_id` text,
          `slug` text NOT NULL,
          `directory` text NOT NULL,
          `path` text,
          `title` text NOT NULL,
          `version` text NOT NULL,
          `share_url` text,
          `summary_additions` integer,
          `summary_deletions` integer,
          `summary_files` integer,
          `summary_diffs` text,
          `metadata` text,
          `cost` real DEFAULT 0 NOT NULL,
          `tokens_input` integer DEFAULT 0 NOT NULL,
          `tokens_output` integer DEFAULT 0 NOT NULL,
          `tokens_reasoning` integer DEFAULT 0 NOT NULL,
          `tokens_cache_read` integer DEFAULT 0 NOT NULL,
          `tokens_cache_write` integer DEFAULT 0 NOT NULL,
          `revert` text,
          `permission` text,
          `agent` text,
          `model` text,
          `time_created` integer NOT NULL,
          `time_updated` integer NOT NULL,
          `time_compacting` integer,
          `time_archived` integer,
          CONSTRAINT `fk_session_project_id_project_id_fk` FOREIGN KEY (`project_id`) REFERENCES `project`(`id`) ON DELETE CASCADE
        );
INSERT INTO session VALUES('ses_synthetic','global',NULL,NULL,'synthetic','/private/tmp/tc-opencode-discovery-probe-ik984gy5','private/tmp/tc-opencode-discovery-probe-ik984gy5','Synthetic export','1.18.29',NULL,NULL,NULL,NULL,NULL,NULL,0.0,0,0,0,0,0,NULL,NULL,NULL,NULL,1788739200000,1788739202000,NULL,NULL);
CREATE TABLE `message` (
          `id` text PRIMARY KEY,
          `session_id` text NOT NULL,
          `time_created` integer NOT NULL,
          `time_updated` integer NOT NULL,
          `data` text NOT NULL,
          CONSTRAINT `fk_message_session_id_session_id_fk` FOREIGN KEY (`session_id`) REFERENCES `session`(`id`) ON DELETE CASCADE
        );
INSERT INTO message VALUES('msg_user','ses_synthetic',1788739200000,1792032388875,'{"role":"user","time":{"created":1788739200000},"agent":"build","model":{"providerID":"fixture","modelID":"model"}}');
INSERT INTO message VALUES('msg_assistant','ses_synthetic',1788739200001,1792032388876,'{"role":"assistant","time":{"created":1788739200001,"completed":1788739202000},"parentID":"msg_user","modelID":"model","providerID":"fixture","mode":"build","agent":"build","path":{"cwd":"/synthetic/project","root":"/synthetic/project"},"cost":0,"tokens":{"input":4,"output":3,"reasoning":0,"cache":{"read":0,"write":0}}}');
CREATE TABLE `part` (
          `id` text PRIMARY KEY,
          `message_id` text NOT NULL,
          `session_id` text NOT NULL,
          `time_created` integer NOT NULL,
          `time_updated` integer NOT NULL,
          `data` text NOT NULL,
          CONSTRAINT `fk_part_message_id_message_id_fk` FOREIGN KEY (`message_id`) REFERENCES `message`(`id`) ON DELETE CASCADE
        );
INSERT INTO part VALUES('prt_question','msg_user','ses_synthetic',1792032388875,1792032388875,'{"type":"text","text":"Run the synthetic check."}');
INSERT INTO part VALUES('prt_reason','msg_assistant','ses_synthetic',1792032388876,1792032388876,'{"type":"reasoning","text":"Check the fixture.","time":{"start":1788739200001,"end":1788739200002}}');
INSERT INTO part VALUES('prt_tool','msg_assistant','ses_synthetic',1792032388876,1792032388876,'{"type":"tool","callID":"call_synthetic","tool":"bash","state":{"status":"completed","input":{"command":"echo synthetic"},"output":"synthetic","title":"Synthetic check","metadata":{},"time":{"start":1788739200002,"end":1788739200003}}}');
INSERT INTO part VALUES('prt_answer','msg_assistant','ses_synthetic',1792032388877,1792032388877,'{"type":"text","text":"Synthetic check completed.","time":{"start":1788739200004,"end":1788739200005}}');
PRAGMA foreign_keys=ON;
