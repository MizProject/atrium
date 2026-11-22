// Copyright (C) 2026 MizProject Philippines
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published
// by the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

//! Shared Atrium legal notices, AGPL-3.0 metadata, and project credits.

/// User-facing application name.
pub const APP_NAME: &str = "Atrium";

/// Verbatim copyright line required for Appropriate Legal Notices.
pub const COPYRIGHT_NOTICE: &str = "Copyright (C) 2026 MizProject Philippines";

/// SPDX license identifier declared by all workspace crates.
pub const LICENSE_ID: &str = "AGPL-3.0-or-later";

/// Human-readable license title.
pub const LICENSE_TITLE: &str = "GNU Affero General Public License v3.0 or later";

/// Official contact email for Atrium / MizProject Philippines.
pub const CONTACT_EMAIL: &str = "mizproject@proton.me";

/// Official Telegram handle for maintainer contact.
pub const CONTACT_TELEGRAM: &str = "@Mizumo_prjkt";

/// Telegram contact URL.
pub const CONTACT_TELEGRAM_URL: &str = "https://t.me/Mizumo_prjkt";

/// Upstream source code repository URL (AGPL Section 13 / Corresponding Source).
pub const REPO_URL: &str = "https://github.com/MizProject/atrium";

/// Upstream GitHub issue tracker URL.
pub const ISSUES_URL: &str = "https://github.com/MizProject/atrium/issues";

/// Canonical GNU Licenses page referenced by AGPLv3 notices.
pub const GNU_LICENSES_URL: &str = "https://www.gnu.org/licenses/";

/// Standard GNU AGPLv3 preamble notice (verbatim terms).
pub const AGPL_V3_PREAMBLE_NOTICE: &str = "\
Copyright (C) 2026 MizProject Philippines

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published
by the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
GNU Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License
along with this program.  If not, see <https://www.gnu.org/licenses/>.";

/// Full GNU Affero General Public License v3 text bundled offline inside Atrium binaries.
pub const LICENSE_FULL_TEXT: &str = include_str!("../../../LICENSE");

/// Concise legal footer printed in CLI help and terminal banners.
pub const CLI_LEGAL_AFTER_HELP: &str = "\
Atrium - Copyright (C) 2026 MizProject Philippines
Licensed under the GNU Affero General Public License v3.0 or later (AGPL-3.0-or-later).
This program comes with ABSOLUTELY NO WARRANTY; see `atrium about` or LICENSE for details.
Source: https://github.com/MizProject/atrium
Contact: mizproject@proton.me | Telegram @Mizumo_prjkt (https://t.me/Mizumo_prjkt)";

/// Formats the full multi-line legal and contact summary for CLI/daemon `--about` output.
pub fn format_about_banner(binary_name: &str, version: &str) -> String {
    format!(
        "{APP_NAME} ({binary_name}) v{version}\n\
         {AGPL_V3_PREAMBLE_NOTICE}\n\n\
         Source repository: {REPO_URL}\n\
         Issue tracker:     {ISSUES_URL}\n\
         Email contact:     {CONTACT_EMAIL}\n\
         Telegram contact:  {CONTACT_TELEGRAM} ({CONTACT_TELEGRAM_URL})\n\n\
         Credits:\n\
         - Project & Reverse-Engineering: MizProject Philippines (@Mizumo_prjkt)\n\
         - Hardware & Community Support: AULA WIN 60 HE & Hall-effect contributors",
        APP_NAME = APP_NAME,
        AGPL_V3_PREAMBLE_NOTICE = AGPL_V3_PREAMBLE_NOTICE,
        REPO_URL = REPO_URL,
        ISSUES_URL = ISSUES_URL,
        CONTACT_EMAIL = CONTACT_EMAIL,
        CONTACT_TELEGRAM = CONTACT_TELEGRAM,
        CONTACT_TELEGRAM_URL = CONTACT_TELEGRAM_URL,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agpl_notices_and_contacts_are_present() {
        assert!(AGPL_V3_PREAMBLE_NOTICE.contains(COPYRIGHT_NOTICE));
        assert!(AGPL_V3_PREAMBLE_NOTICE.contains("GNU Affero General Public License"));
        assert!(AGPL_V3_PREAMBLE_NOTICE.contains(GNU_LICENSES_URL));
        assert!(LICENSE_FULL_TEXT.contains("GNU AFFERO GENERAL PUBLIC LICENSE"));
        assert!(LICENSE_FULL_TEXT.contains("Version 3, 19 November 2007"));
        assert!(CLI_LEGAL_AFTER_HELP.contains(CONTACT_EMAIL));
        assert!(CLI_LEGAL_AFTER_HELP.contains(CONTACT_TELEGRAM));
        assert!(CLI_LEGAL_AFTER_HELP.contains(REPO_URL));
        let banner = format_about_banner("atrium", "0.1.0");
        assert!(banner.contains(COPYRIGHT_NOTICE));
        assert!(banner.contains(CONTACT_EMAIL));
    }
}
