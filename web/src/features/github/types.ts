// GraphQL fields selected by core/items.rs. Connections retain GitHub's field names.
export type Label = { name: string; color: string };
export type User = { login?: string; name?: string; slug?: string; avatarUrl?: string };
export type Connection<T> = { nodes: T[]; pageInfo?: { hasNextPage: boolean } };
export type Comment = {
  id: string;
  body: string;
  bodyHTML: string;
  url: string;
  createdAt?: string;
  submittedAt?: string;
  author?: User;
  state?: string;
};
export type Timeline = Partial<Comment> & {
  __typename: string;
  actor?: User;
  label?: { name: string };
  assignee?: User;
  requestedReviewer?: User;
  previousTitle?: string;
  currentTitle?: string;
  source?: { title: string; url: string };
  commit?: {
    oid: string;
    messageHeadline: string;
    committedDate: string;
    url: string;
    author: User;
  };
};
export type Item = {
  __typename: "Issue" | "PullRequest";
  id: string;
  repo: string;
  number: number;
  title: string;
  url: string;
  body: string;
  bodyHTML: string;
  state: string;
  createdAt: string;
  updatedAt: string;
  fetched_at: number;
  author: User;
  labels: Connection<Label>;
  assignees: Connection<User>;
  participants: Connection<User>;
  milestone?: { title: string };
  timelineItems: Connection<Timeline>;
  isDraft?: boolean;
  merged?: boolean;
  mergeable?: string;
  mergeStateStatus?: string;
  reviewDecision?: string;
  headRefName?: string;
  baseRefName?: string;
  headRefOid?: string;
  baseRefOid?: string;
  repository?: {
    mergeCommitAllowed: boolean;
    squashMergeAllowed: boolean;
    rebaseMergeAllowed: boolean;
  };
  reviewRequests?: Connection<{ requestedReviewer: User }>;
  reviews?: Connection<{ state: string; author: User }>;
  reviewThreads?: Connection<{
    id: string;
    path: string;
    line: number | null;
    isResolved: boolean;
    isOutdated: boolean;
    comments: Connection<Comment>;
  }>;
  closingIssuesReferences?: Connection<{
    number: number;
    title: string;
    url: string;
  }>;
  commits?: Connection<{
    commit: {
      statusCheckRollup?: {
        state: string;
        contexts: Connection<{
          name?: string;
          context?: string;
          conclusion?: string;
          status?: string;
          state?: string;
          detailsUrl?: string;
          targetUrl?: string;
        }>;
      };
    };
  }>;
};
export type ListItem = {
  labels?: Connection<Label>;
  number: number;
  title: string;
  url: string;
  state: string;
  isDraft?: boolean;
  updatedAt: string;
  author: User;
};
export type RepoItems = {
  fetched_at: number;
  prs: ListItem[];
  issues: ListItem[];
};
export type DiffFile = {
  filename: string;
  previous_filename?: string;
  status: string;
  patch?: string;
  language?: string;
};
export type Diff = {
  files: DiffFile[];
  project_id: string | null;
  head: string;
  base: string;
  fallback: boolean;
  local_error?: string;
};
