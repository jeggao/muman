"""yt-dlp plugin that makes a YouTube Music track's album art its thumbnail.

A track's own thumbnails are 16:9 frames with the square art pillarboxed in
the middle. The square art is listed only by the `web_music` player client,
on googleusercontent.com, ranked below every frame, and at most 544 pixels.
"""

import re

from yt_dlp.postprocessor.common import PostProcessor

# The size directive after the last `=`: `s0` asks for the art at its own
# size, `rj` for JPEG whatever it was uploaded as.
_SIZE = re.compile(r"=[^=/]*$")


def _album_art(info):
    if not info.get("track"):
        return None
    for t in reversed(info.get("thumbnails") or []):
        url = t.get("url", "")
        square = t.get("width") and t.get("width") == t.get("height")
        if "googleusercontent.com/" in url and square and _SIZE.search(url):
            return _SIZE.sub("=s0-rj", url)
    return None


class AlbumArtPP(PostProcessor):
    def run(self, info):
        url = _album_art(info)
        if url:
            # The last thumbnail is the one written and embedded.
            info["thumbnails"] = [
                *(info.get("thumbnails") or []),
                {"id": "album_art", "url": url, "ext": "jpg"},
            ]
        return [], info
