CREATE TABLE reports (
  id TEXT PRIMARY KEY,
  kind TEXT NOT NULL CHECK (kind IN ('bug', 'feedback')),
  message TEXT NOT NULL,
  contact TEXT,
  specs TEXT NOT NULL,
  has_logs INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL,
  issue_url TEXT
);
