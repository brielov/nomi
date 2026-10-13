//! The licenses a package may have (`Elm/Licenses.hs`): the OSI-approved
//! ones, by SPDX identifier, as upstream lists them.

use nomi_base::{Compat, suggest};

use crate::scan;

/// A license from [`LICENSES`], as the raw bytes of its JSON string.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct License(Box<[u8]>);

impl License {
    pub fn bsd3() -> License {
        License(b"BSD-3-Clause".as_slice().into())
    }

    /// The raw string bytes, as `encode` writes them.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// `check`: the license these raw string bytes name, or the four
    /// identifiers nearest to them. Escapes are not decoded, so `MI\u0054`
    /// is no license. An identifier can be suggested twice, once for
    /// itself and once for its full name.
    pub fn check(raw: &[u8], compat: Compat) -> Result<License, Vec<&'static str>> {
        if LICENSES
            .binary_search_by(|(code, _)| code.as_bytes().cmp(raw))
            .is_ok()
        {
            return Ok(License(raw.into()));
        }
        let mut candidates: Vec<(&'static str, &'static str)> = LICENSES
            .iter()
            .map(|&(code, _)| (code, code))
            .chain(LICENSES.iter().copied())
            .collect();
        suggest::sort(&code_points(raw, compat), &mut candidates, |(_, s)| {
            suggest::code_points(s)
        });
        Err(candidates
            .into_iter()
            .take(4)
            .map(|(code, _)| code)
            .collect())
    }
}

/// `Utf8.toChars`: 0.19.1 assembles each character from its lead byte's
/// width unchecked, where later versions hold valid UTF-8.
fn code_points(raw: &[u8], compat: Compat) -> Vec<u32> {
    match compat {
        Compat::V0_19_2 | Compat::V0_19_3 => String::from_utf8_lossy(raw)
            .chars()
            .map(u32::from)
            .collect(),
        // Wrapped, as only equality matters and no two codes collide.
        Compat::V0_19_1 => scan::unchecked_chars(raw)
            .into_iter()
            .map(|code| code as u32)
            .collect(),
    }
}

/// SPDX identifiers and full names, in the identifiers' byte order.
pub const LICENSES: &[(&str, &str)] = &[
    ("0BSD", "BSD Zero Clause License"),
    ("AAL", "Attribution Assurance License"),
    ("AFL-1.1", "Academic Free License v1.1"),
    ("AFL-1.2", "Academic Free License v1.2"),
    ("AFL-2.0", "Academic Free License v2.0"),
    ("AFL-2.1", "Academic Free License v2.1"),
    ("AFL-3.0", "Academic Free License v3.0"),
    ("AGPL-3.0", "GNU Affero General Public License v3.0"),
    ("APL-1.0", "Adaptive Public License 1.0"),
    ("APSL-1.0", "Apple Public Source License 1.0"),
    ("APSL-1.1", "Apple Public Source License 1.1"),
    ("APSL-1.2", "Apple Public Source License 1.2"),
    ("APSL-2.0", "Apple Public Source License 2.0"),
    ("Apache-1.1", "Apache License 1.1"),
    ("Apache-2.0", "Apache License 2.0"),
    ("Artistic-1.0", "Artistic License 1.0"),
    ("Artistic-1.0-Perl", "Artistic License 1.0 (Perl)"),
    ("Artistic-1.0-cl8", "Artistic License 1.0 w/clause 8"),
    ("Artistic-2.0", "Artistic License 2.0"),
    ("BSD-2-Clause", "BSD 2-clause \"Simplified\" License"),
    (
        "BSD-3-Clause",
        "BSD 3-clause \"New\" or \"Revised\" License",
    ),
    ("BSL-1.0", "Boost Software License 1.0"),
    (
        "CATOSL-1.1",
        "Computer Associates Trusted Open Source License 1.1",
    ),
    (
        "CDDL-1.0",
        "Common Development and Distribution License 1.0",
    ),
    ("CECILL-2.1", "CeCILL Free Software License Agreement v2.1"),
    ("CNRI-Python", "CNRI Python License"),
    ("CPAL-1.0", "Common Public Attribution License 1.0"),
    ("CPL-1.0", "Common Public License 1.0"),
    ("CUA-OPL-1.0", "CUA Office Public License v1.0"),
    ("ECL-1.0", "Educational Community License v1.0"),
    ("ECL-2.0", "Educational Community License v2.0"),
    ("EFL-1.0", "Eiffel Forum License v1.0"),
    ("EFL-2.0", "Eiffel Forum License v2.0"),
    ("EPL-1.0", "Eclipse Public License 1.0"),
    ("EUDatagrid", "EU DataGrid Software License"),
    ("EUPL-1.1", "European Union Public License 1.1"),
    ("Entessa", "Entessa Public License v1.0"),
    ("Fair", "Fair License"),
    ("Frameworx-1.0", "Frameworx Open License 1.0"),
    ("GPL-2.0", "GNU General Public License v2.0 only"),
    ("GPL-3.0", "GNU General Public License v3.0 only"),
    ("HPND", "Historic Permission Notice and Disclaimer"),
    ("IPA", "IPA Font License"),
    ("IPL-1.0", "IBM Public License v1.0"),
    ("ISC", "ISC License"),
    ("Intel", "Intel Open Source License"),
    ("LGPL-2.0", "GNU Library General Public License v2 only"),
    ("LGPL-2.1", "GNU Lesser General Public License v2.1 only"),
    ("LGPL-3.0", "GNU Lesser General Public License v3.0 only"),
    ("LPL-1.0", "Lucent Public License Version 1.0"),
    ("LPL-1.02", "Lucent Public License v1.02"),
    ("LPPL-1.3c", "LaTeX Project Public License v1.3c"),
    (
        "LiLiQ-P-1.1",
        "Licence Libre du Québec – Permissive version 1.1",
    ),
    (
        "LiLiQ-R-1.1",
        "Licence Libre du Québec – Réciprocité version 1.1",
    ),
    (
        "LiLiQ-Rplus-1.1",
        "Licence Libre du Québec – Réciprocité forte version 1.1",
    ),
    ("MIT", "MIT License"),
    ("MPL-1.0", "Mozilla Public License 1.0"),
    ("MPL-1.1", "Mozilla Public License 1.1"),
    ("MPL-2.0", "Mozilla Public License 2.0"),
    (
        "MPL-2.0-no-copyleft-exception",
        "Mozilla Public License 2.0 (no copyleft exception)",
    ),
    ("MS-PL", "Microsoft Public License"),
    ("MS-RL", "Microsoft Reciprocal License"),
    ("MirOS", "MirOS Licence"),
    ("Motosoto", "Motosoto License"),
    ("Multics", "Multics License"),
    ("NASA-1.3", "NASA Open Source Agreement 1.3"),
    ("NCSA", "University of Illinois/NCSA Open Source License"),
    ("NGPL", "Nethack General Public License"),
    ("NPOSL-3.0", "Non-Profit Open Software License 3.0"),
    ("NTP", "NTP License"),
    ("Naumen", "Naumen Public License"),
    ("Nokia", "Nokia Open Source License"),
    ("OCLC-2.0", "OCLC Research Public License 2.0"),
    ("OFL-1.1", "SIL Open Font License 1.1"),
    ("OGTSL", "Open Group Test Suite License"),
    ("OSET-PL-2.1", "OSET Public License version 2.1"),
    ("OSL-1.0", "Open Software License 1.0"),
    ("OSL-2.0", "Open Software License 2.0"),
    ("OSL-2.1", "Open Software License 2.1"),
    ("OSL-3.0", "Open Software License 3.0"),
    ("PHP-3.0", "PHP License v3.0"),
    ("PostgreSQL", "PostgreSQL License"),
    ("Python-2.0", "Python License 2.0"),
    ("QPL-1.0", "Q Public License 1.0"),
    ("RPL-1.1", "Reciprocal Public License 1.1"),
    ("RPL-1.5", "Reciprocal Public License 1.5"),
    ("RPSL-1.0", "RealNetworks Public Source License v1.0"),
    ("RSCPL", "Ricoh Source Code Public License"),
    ("SISSL", "Sun Industry Standards Source License v1.1"),
    ("SPL-1.0", "Sun Public License v1.0"),
    ("SimPL-2.0", "Simple Public License 2.0"),
    ("Sleepycat", "Sleepycat License"),
    ("UPL-1.0", "Universal Permissive License v1.0"),
    ("VSL-1.0", "Vovida Software License v1.0"),
    ("W3C", "W3C Software Notice and License (2002-12-31)"),
    ("Watcom-1.0", "Sybase Open Watcom Public License 1.0"),
    ("Xnet", "X.Net License"),
    ("ZPL-2.0", "Zope Public License 2.0"),
    ("Zlib", "zlib License"),
];
