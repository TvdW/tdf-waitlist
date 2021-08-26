use std::{collections::BTreeMap, num::ParseIntError};

use crate::{Type, TypeError};

use super::{Category, TypeDB, TypeID};

#[derive(Debug)]
pub struct Fitting {
    pub hull: TypeID,
    pub modules: BTreeMap<TypeID, i64>,
    pub cargo: BTreeMap<TypeID, i64>,
}

#[derive(Debug)]
pub enum FitError {
    InvalidFit,
    InvalidModule,
}

impl From<ParseIntError> for FitError {
    fn from(_: ParseIntError) -> Self {
        FitError::InvalidFit
    }
}

impl From<TypeError> for FitError {
    fn from(_: TypeError) -> Self {
        FitError::InvalidModule
    }
}

impl Fitting {
    pub fn from_dna(dna: &str) -> Result<Fitting, FitError> {
        let mut pieces = dna.split(':');
        let hull: TypeID = pieces.next().unwrap().parse()?; // 1st elmt

        let mut cargo = BTreeMap::new();
        let mut modules = BTreeMap::new();

        let mut i = 0;
        for piece in pieces {
            if piece.is_empty() {
                continue;
            }
            i += 1;
            if i > 1000 {
                return Err(FitError::InvalidFit);
            }

            let mut mod_split = piece.splitn(2, ';');
            let type_id_str = mod_split.next().unwrap(); // 1st elmt
            let type_id: TypeID = if type_id_str.ends_with('_') {
                type_id_str.strip_suffix('_').unwrap().parse()?
            } else {
                type_id_str.parse()?
            };

            let count_str = mod_split.next();
            let count = match count_str {
                None => 1,
                Some(i) => i.parse()?,
            };

            let is_cargo = if type_id_str.ends_with('_') {
                true
            } else {
                let loaded_type = TypeDB::load_type(type_id)?;
                Self::is_always_cargo(&loaded_type)
            };

            let desto = match is_cargo {
                true => &mut cargo,
                false => &mut modules,
            };

            *desto.entry(type_id).or_insert(0) += count;
        }

        Ok(Fitting {
            hull,
            cargo,
            modules,
        })
    }

    pub fn to_dna(&self) -> Result<String, FitError> {
        let mut dna = format!("{}:", self.hull);

        for (id, count) in &self.modules {
            // XXX We could/should sort this by slot
            dna += &format!("{};{}:", id, count);
        }

        for (&id, &count) in &self.cargo {
            let inv_type = TypeDB::load_type(id)?;
            if Self::is_always_cargo(&inv_type) {
                dna += &format!("{};{}:", id, count);
            } else {
                dna += &format!("{}_;{}:", id, count);
            }
        }

        Ok(dna + ":")
    }

    fn is_always_cargo(what: &Type) -> bool {
        what.category == Category::Charge || what.category == Category::Implant
    }

    pub fn from_eft(eft: &str) -> Result<Vec<Fitting>, FitError> {
        let mut fittings = Vec::new();
        let mut section = 0;
        for line in eft.lines() {
            let line = line.trim();

            if line.starts_with('[') && line.ends_with(']') && line.contains(',') {
                let line = line.strip_prefix('[').unwrap().strip_suffix(']').unwrap();
                let mut pieces = line.splitn(2, ',');
                let hull_name = pieces.next().unwrap().trim(); // 1st elmt
                let ship_name = pieces.next();
                if ship_name.is_none() {
                    return Err(FitError::InvalidFit);
                }
                let hull = TypeDB::id_of(hull_name)?;
                fittings.push(Fitting {
                    hull,
                    cargo: BTreeMap::new(),
                    modules: BTreeMap::new(),
                });
                section = 0;
            } else if let Some(fit) = fittings.last_mut() {
                if line.starts_with("[Empty ") {
                    continue;
                }
                if line.is_empty() {
                    section += 1;
                } else {
                    let mut pieces = line.split(" x");
                    let type_name = pieces.next().unwrap(); // 1st elmt
                    let type_id = TypeDB::id_of(type_name)?;

                    let (count, stacked) = match pieces.next() {
                        None => (1, false),
                        Some(s) => (s.parse()?, true),
                    };

                    let is_cargo = if section >= 7 {
                        // Sections are high,med,low,rig,subsystem,drone, then cargo
                        true
                    } else {
                        let type_obj = TypeDB::load_type(type_id)?;
                        Self::is_always_cargo(&type_obj)
                            || (stacked && type_obj.category != Category::Drone)
                    };

                    let desto = if is_cargo {
                        &mut fit.cargo
                    } else {
                        &mut fit.modules
                    };

                    *desto.entry(type_id).or_insert(0) += count;
                }
            } else {
                return Err(FitError::InvalidFit);
            }
        }

        Ok(fittings)
    }
}

#[cfg(test)]
mod tests {
    use super::Fitting;

    #[test]
    fn test_parse_dna() {
        let parsed = Fitting::from_dna("670::").unwrap();
        assert_eq!(parsed.hull, 670);
        assert!(parsed.modules.is_empty());
        assert!(parsed.cargo.is_empty());
        assert_eq!(parsed.to_dna().unwrap(), "670::");

        let parsed = Fitting::from_dna("59630:24554;1:23674;1:24550;1:24552;1::").unwrap();
        assert_eq!(parsed.hull, 59630);
        assert!(parsed.cargo.is_empty());
        assert_eq!(*parsed.modules.get(&24554).unwrap(), 1);
        assert_eq!(*parsed.modules.get(&23674).unwrap(), 1);
        assert_eq!(*parsed.modules.get(&24550).unwrap(), 1);
        assert_eq!(*parsed.modules.get(&24552).unwrap(), 1);
        assert_eq!(parsed.modules.len(), 4);
        assert_eq!(
            parsed.to_dna().unwrap(),
            "59630:23674;1:24550;1:24552;1:24554;1::"
        );

        let parsed = Fitting::from_dna("17736:3057;4:12816;2:4383_;1::").unwrap();
        assert_eq!(parsed.hull, 17736);
        assert_eq!(parsed.modules.len(), 1);
        assert_eq!(parsed.cargo.len(), 2);
        assert_eq!(*parsed.modules.get(&3057).unwrap(), 4);
        assert_eq!(*parsed.cargo.get(&12816).unwrap(), 2);
        assert_eq!(*parsed.cargo.get(&4383).unwrap(), 1);
        assert_eq!(parsed.to_dna().unwrap(), "17736:3057;4:4383_;1:12816;2::");
    }

    #[test]
    fn test_parse_eft() {
        let parsed = Fitting::from_eft(
            "[Nightmare, Nightmare]


Mega Pulse Laser II
Mega Pulse Laser II
Mega Pulse Laser II
Mega Pulse Laser II





Conflagration L x2
Large Micro Jump Drive x1
",
        )
        .unwrap();
        assert_eq!(parsed.len(), 1);
        let parsed = parsed.into_iter().next().unwrap();
        assert_eq!(parsed.hull, 17736);
        assert_eq!(parsed.modules.len(), 1);
        assert_eq!(parsed.cargo.len(), 2);
        assert_eq!(*parsed.modules.get(&3057).unwrap(), 4);
        assert_eq!(*parsed.cargo.get(&12816).unwrap(), 2);
        assert_eq!(*parsed.cargo.get(&4383).unwrap(), 1);
        assert_eq!(parsed.to_dna().unwrap(), "17736:3057;4:4383_;1:12816;2::");
    }

    #[test]
    fn test_parse_eft_2() {
        let parsed = Fitting::from_eft(
            "[Venture, *Simulated Venture Fitting]






Hobgoblin II x2

1600mm Steel Plates II x1

",
        )
        .unwrap()
        .pop()
        .expect("Parsed");
        assert_eq!(*parsed.modules.get(&2456).unwrap(), 2);
        assert_eq!(*parsed.cargo.get(&20353).unwrap(), 1);
    }
}
