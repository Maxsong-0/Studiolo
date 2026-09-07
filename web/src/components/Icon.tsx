type IconName =
  | 'overview'
  | 'inbox'
  | 'document'
  | 'lock'
  | 'folder'
  | 'refresh'
  | 'search'
  | 'close'
  | 'arrow'
  | 'check'
  | 'decision'
  | 'image'
  | 'table'
  | 'code'
  | 'canvas'
  | 'attachment'
  | 'book'
  | 'account'
  | 'logout'
  | 'shield'
  | 'key'
  | 'copy'
  | 'chevron';

const paths: Record<IconName, React.ReactNode> = {
  overview: (
    <>
      <rect x="3" y="3" width="7" height="7" />
      <rect x="14" y="3" width="7" height="7" />
      <rect x="3" y="14" width="7" height="7" />
      <rect x="14" y="14" width="7" height="7" />
    </>
  ),
  inbox: (
    <>
      <rect x="3" y="3" width="18" height="18" rx="1" />
      <path d="m7 12 3 3 7-7" />
    </>
  ),
  document: (
    <>
      <path d="M5 2h9l5 5v15H5zM14 2v6h5M9 12h6M9 16h6" />
    </>
  ),
  lock: (
    <>
      <rect x="4" y="10" width="16" height="12" rx="1" />
      <path d="M8 10V6a4 4 0 0 1 8 0v4M12 15v3" />
    </>
  ),
  folder: <path d="M3 4h6l3 3h9v14H3z" />,
  refresh: (
    <>
      <path d="M20 8a8 8 0 0 0-14-3L3 8m0-5v5h5M4 16a8 8 0 0 0 14 3l3-3m0 5v-5h-5" />
    </>
  ),
  search: (
    <>
      <circle cx="10.5" cy="10.5" r="7" />
      <path d="m16 16 5 5" />
    </>
  ),
  close: <path d="m6 6 12 12M6 18 18 6" />,
  arrow: <path d="M4 12h16m-6-6 6 6-6 6" />,
  check: (
    <>
      <circle cx="12" cy="12" r="9" />
      <path d="m8 12 3 3 5-6" />
    </>
  ),
  decision: (
    <>
      <path d="M12 21V11m0 0L6 5m6 6 6-6M3 5h6M6 2v6m9-3h6m-3-3v6" />
    </>
  ),
  image: (
    <>
      <rect x="3" y="3" width="18" height="18" rx="3" />
      <circle cx="8" cy="8" r="1.5" />
      <path d="m3 17 5-5 4 4 4-6 5 7" />
    </>
  ),
  table: (
    <>
      <rect x="3" y="3" width="18" height="18" rx="2" />
      <path d="M3 9h18M9 9v12m6-12v12M3 15h18" />
    </>
  ),
  code: (
    <>
      <path d="M8 7 3 12l5 5m8-10 5 5-5 5m-3-14-2 18" />
    </>
  ),
  canvas: (
    <>
      <rect x="3" y="3" width="6" height="6" rx="1" />
      <rect x="15" y="15" width="6" height="6" rx="1" />
      <path d="M9 6h6a3 3 0 0 1 3 3v6M6 9v6a3 3 0 0 0 3 3h6" />
    </>
  ),
  attachment: (
    <path d="m9 13 6-6a3 3 0 0 1 4 4l-8 8a5 5 0 0 1-7-7l8-8m-4 12 8-8" />
  ),
  book: (
    <>
      <path d="M3 4h6a4 4 0 0 1 3 2 4 4 0 0 1 3-2h6v16h-6a4 4 0 0 0-3 1 4 4 0 0 0-3-1H3zM12 6v15" />
    </>
  ),
  chevron: <path d="m6 9 6 6 6-6" />,
  shield: (
    <>
      <path d="m12 3 8 3v6c0 5-8 9-8 9s-8-4-8-9V6z" />
      <path d="m8 12 3 3 5-6" />
    </>
  ),
  key: (
    <>
      <circle cx="8" cy="8" r="5" />
      <path d="m12 12 9 9m-6-6 3-3m0 6 3-3" />
    </>
  ),
  copy: (
    <>
      <rect x="8" y="8" width="13" height="13" rx="2" />
      <path d="M16 8V3H3v13h5" />
    </>
  ),
  account: (
    <>
      <circle cx="12" cy="8" r="4" />
      <path d="M4 21v-2a8 8 0 0 1 16 0v2" />
    </>
  ),
  logout: (
    <>
      <path d="M10 3H4v18h6M9 12h12m-4-4 4 4-4 4" />
    </>
  ),
};

export function Icon({
  name,
  className = '',
}: {
  name: IconName;
  className?: string;
}) {
  return (
    <svg
      className={`icon ${className}`}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.7"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      {paths[name]}
    </svg>
  );
}
