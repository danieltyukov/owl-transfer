import { ChevronGlyph } from '../../icons/glyphs.js';

export interface BreadcrumbProps {
  /** Relative to the sync folder. The empty string is the folder itself. */
  dir: string;
  /** What the folder is called, shown as the first crumb. */
  root: string;
  onDir: (dir: string) => void;
}

/*
 * Where in the folder we are, and the way back out.
 *
 * The first crumb is the sync folder's own name rather than a home glyph, so
 * the trail reads as a path a person could type. Every crumb but the last is a
 * button; the last is the heading for what is on screen.
 */
export function Breadcrumb({ dir, root, onDir }: BreadcrumbProps) {
  const parts = dir === '' ? [] : dir.split('/');
  const crumbs = [{ label: root, path: '' }];
  for (const [index, part] of parts.entries()) {
    crumbs.push({ label: part, path: parts.slice(0, index + 1).join('/') });
  }

  return (
    <nav className="crumbs" aria-label="Folder">
      {crumbs.map((crumb, index) => {
        const last = index === crumbs.length - 1;
        return (
          <span className="crumb-part" key={crumb.path}>
            {index === 0 ? null : (
              <span className="crumb-sep" aria-hidden="true">
                <ChevronGlyph />
              </span>
            )}
            {last ? (
              <h1 className="crumb crumb-here">{crumb.label}</h1>
            ) : (
              <button type="button" className="crumb" onClick={() => onDir(crumb.path)}>
                {crumb.label}
              </button>
            )}
          </span>
        );
      })}
    </nav>
  );
}
