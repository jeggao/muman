"""yt-dlp plugin that gives each file of an archive.org item the item's details.

A file's own address yields one file, which yt-dlp gives the item's creator,
dates, licence and description. An item of several yields a playlist whose
files carry only what each file's own entry says, so one file read either way
reads two ways. Each file here takes the item's details where it has none of
its own, as one file does.
"""

from yt_dlp.utils import merge_dicts

try:
    from yt_dlp.extractor.archiveorg import ArchiveOrgIE
except ImportError:
    ArchiveOrgIE = None

if ArchiveOrgIE is not None:

    class _ArchiveItemIE(ArchiveOrgIE, plugin_name="muman"):
        def _real_extract(self, url):
            info = super()._real_extract(url)
            if info.get("_type") != "playlist":
                return info
            item = {k: v for k, v in info.items() if k not in ("_type", "entries")}
            info["entries"] = [merge_dicts(e, item) for e in info["entries"]]
            return info
