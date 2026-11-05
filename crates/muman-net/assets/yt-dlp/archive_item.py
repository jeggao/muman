"""yt-dlp plugin that gives each file of an archive.org item the item's details.

A file's own address yields one file, which yt-dlp gives the item's creator,
dates, licence and description. An item of several yields a playlist whose
files carry only what each file's own entry says, so one file read either way
reads two ways. Each file here takes the item's details where it has none of
its own, as one file does.

A file's download address, `/download/<item>/<file>`, names the file its
details address does, and is handed to the item's extractor as that one:
yt-dlp otherwise reads it as a plain file of the mirror it redirects to,
without the item's details, named by the file alone.
"""

from yt_dlp.extractor.common import InfoExtractor
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

    class ArchiveDownloadIE(InfoExtractor):
        _VALID_URL = r"https?://(?:www\.)?archive\.org/download/(?P<id>[^/?#]+/[^?#]+)"

        def _real_extract(self, url):
            # A path's `+` is itself, where the details address reads one as a space.
            path = self._match_id(url).replace("+", "%2B")
            return self.url_result(f"https://archive.org/details/{path}", ArchiveOrgIE.ie_key())
