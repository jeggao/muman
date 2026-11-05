"""yt-dlp plugin that drops machine-translated captions before download.

YouTube lists a translation of its speech-recognition track into every
supported language, and `--sub-langs` cannot tell those apart from human
subtitles sharing the same code. Only translations carry `tlang` in their URL.
"""

from urllib.parse import parse_qs, urlparse

from yt_dlp.postprocessor.common import PostProcessor


def _is_translation(sub):
    return "tlang" in parse_qs(urlparse(sub.get("url", "")).query)


class OriginalSubsPP(PostProcessor):
    def run(self, info):
        subs = info.get("requested_subtitles")
        if not subs:
            return [], info
        # YouTube lists the original track twice, as `<lang>` and
        # `<lang>-orig`; the bare code is the one that maps to a language tag.
        plain_urls = {
            sub.get("url") for lang, sub in subs.items() if not lang.endswith("-orig")
        }
        info["requested_subtitles"] = {
            lang: sub
            for lang, sub in subs.items()
            if not _is_translation(sub)
            and not (lang.endswith("-orig") and sub.get("url") in plain_urls)
        }
        return [], info
