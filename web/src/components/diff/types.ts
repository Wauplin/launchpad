// A diff of Local PR, commit or session changes, as served by the daemon.
export type DiffFile = {
  filename: string;
  previous_filename?: string;
  status: string;
  patch?: string;
  additions?: number;
  deletions?: number;
  language?: string;
};
export type Diff = {
  files: DiffFile[];
  project_id: string | null;
  head: string;
  base: string;
};
