// Which splat container a file is, as a file name Spark can detect from. `.splat` has no magic
// number, so Spark needs the extension; the core already identified the format from the
// content (`meta.format`), so a mislabelled file is still decoded as what it really is.

export const SPLAT_FORMATS = ['splat', 'ply', 'spz'];

/** `asset.<format>` from the core's content-based `format`, else from the path's extension. */
export const splatFileName = (format, path = '') => {
  if (SPLAT_FORMATS.includes(format)) return `asset.${format}`;
  const extension = path.split('.').pop()?.toLowerCase();
  return SPLAT_FORMATS.includes(extension) ? `asset.${extension}` : 'asset.splat';
};
