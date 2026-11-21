import pkg from '../../package.json';
import changelogMarkdown from '../../CHANGELOG.md?raw';

export const APP_SEMVER: string = pkg.version;

export function formatVersionBadge(semver: string): string {
  const parts = semver.split('.');
  if (parts.length === 3 && parts[2] === '0') {
    return `v${parts[0]}.${parts[1]}`;
  }
  return `v${semver}`;
}

export const APP_VERSION_LABEL: string = formatVersionBadge(APP_SEMVER);
export const CHANGELOG_MARKDOWN: string = changelogMarkdown;
