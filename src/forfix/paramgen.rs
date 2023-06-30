use std::io::{Error, ErrorKind, Read, Result, Write};
use ff::{PrimeField, Field};
use pairing_plus::{bls12_381::{Fq12, G1Affine, G2Affine, self, Fr, FrRepr}, serdes::SerDes, CurveAffine, CurveProjective};
use rand::rngs::OsRng;
use rand::RngCore;
use super::hash_to_field_pointproofs::*;
use zeroize::Zeroize;

#[derive(Debug, PartialEq)]
pub struct PointproofsParams {
    /// parameter N
    pub n: usize,

    /// g1^{alpha}, ..., g1^{alpha^N}
    pub g1_alpha_1_to_n: Vec<G1Affine>, //[G1Affine; N],

    /// g1^{alpha^{N+2}}, g1^{alpha^{N+3}}, ..., g1^{alpha^{2N}}
    pub g1_alpha_nplus2_to_2n: Vec<G1Affine>, //[G1Affine; N - 1],

    /// g2^{alpha}, g2^{alpha^2}, ..., g2^{alpha^N}
    pub g2_alpha_1_to_n: Vec<G2Affine>, // [G2Affine; N],

    /// g2^{alpha^{N+2}}, g2^{alpha^{N+3}}, ..., g2^{alpha^{2N}}
    pub g2_alpha_nplus2_to_2n: Vec<G2Affine>, // [G2Affine; N - 1],

    /// e(g2,g1)^{alpha^{N+1}}
    pub gt_alpha_nplus1: Fq12,
}

impl SerDes for PointproofsParams {
    fn serialize<W: Write>(&self, w: &mut W, compressed: bool) -> Result<()> {
        if !compressed {
            return Err(Error::new(
                ErrorKind::Other,
                "Pointproofs params can only be (de)serialized with compressed=true",
            ));
        }
        if self.n > (u32::max_value() as usize) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Invalid n",
            ));
        }
        w.write_all(&(self.n as u32).to_le_bytes())?;
        for pt in &self.g1_alpha_1_to_n {
            pt.serialize(w, true)?;
        }
        for pt in &self.g1_alpha_nplus2_to_2n {
            pt.serialize(w, true)?;
        }
        for pt in &self.g2_alpha_1_to_n {
            pt.serialize(w, true)?;
        }
        for pt in &self.g2_alpha_nplus2_to_2n {
            pt.serialize(w, true)?;
        }
        self.gt_alpha_nplus1.serialize(w, true)?;
        Ok(())
    }
    
    fn deserialize<R: Read>(r: &mut R, compressed: bool) -> Result<Self> {
        if !compressed {
            return Err(Error::new(
                ErrorKind::Other,
                "Pointproofs params can only be (de)serialized with compressed=true",
            ));
        }

        // read parameter n
        let mut buf = [0u8; 4];
        r.read_exact(&mut buf)?;
        let n = u32::from_le_bytes(buf) as usize;

        if n > 65535 || n == 0 {
            return Err(Error::new(
                ErrorKind::Other,
                "The size of n has passed the maximal allowed value.",
            ));
        }

        let mut g1_alpha_1_to_n: Vec<G1Affine> = vec![];
        let mut g1_alpha_nplus2_to_2n: Vec<G1Affine> = vec![];
        let mut g2_alpha_1_to_n: Vec<G2Affine> = vec![];
        let mut g2_alpha_nplus2_to_2n: Vec<G2Affine> = vec![];
        let gt_alpha_nplus1: Fq12;

        for _ in 0..n {
            let tmp = G1Affine::deserialize(r, true)?;
            g1_alpha_1_to_n.push(tmp);
        }
        for _ in 0..n - 1 {
            let tmp = G1Affine::deserialize(r, true)?;
            g1_alpha_nplus2_to_2n.push(tmp);
        }
        for _ in 0..n {
            let tmp = G2Affine::deserialize(r, true)?;
            g2_alpha_1_to_n.push(tmp);
        }
        for _ in 0..n - 1 {
            let tmp = G2Affine::deserialize(r, true)?;
            g2_alpha_nplus2_to_2n.push(tmp);
        }

        gt_alpha_nplus1 = Fq12::deserialize(r, true)?;

        Ok(PointproofsParams {
            n,
            g1_alpha_1_to_n,
            g1_alpha_nplus2_to_2n,
            g2_alpha_1_to_n,
            g2_alpha_nplus2_to_2n,
            gt_alpha_nplus1,
        })
    }
}

fn random_scalar() -> Fr {
    let mut r: [u8; 64] = [0; 64];
    // @dev: 下面这两行代码取代了原始代码中的 OsRng {}.fill_bytes(&mut r[..]);
    let mut rng = OsRng::new().expect("Failed to create OsRng instance");
    rng.fill_bytes(&mut r[..]);
    // OsRng {}.fill_bytes(&mut r[..]); // 原始代码, 来自 rand_core::OsRng
    // For convenience, just using already-implemented hash-to-field
    let res = hash_to_field_pointproofs(&r[..]);
    r.zeroize();
    res
}

// Checks that a set of parameters are in the correct form (g2^alpha, g2^alpha^2, etc.) for some alpha
pub fn consistent(params: &PointproofsParams) -> bool {
    // First, check all points are in the group, nonzero, and not the generator
    // (Subgroup check is already done in our deserialization code)
    if params
        .g2_alpha_1_to_n
        .iter()
        .any(|&x| x == G2Affine::zero() || x == G2Affine::one())
    {
        return false;
    }

    if params
        .g2_alpha_nplus2_to_2n
        .iter()
        .any(|&x| x == G2Affine::zero() || x == G2Affine::one())
    {
        return false;
    }

    if params
        .g1_alpha_1_to_n
        .iter()
        .any(|&x| x == G1Affine::zero() || x == G1Affine::one())
    {
        return false;
    }

    if params
        .g1_alpha_nplus2_to_2n
        .iter()
        .any(|&x| x == G1Affine::zero() || x == G1Affine::one())
    {
        return false;
    }

    // Generate N random scalars r_1, ..., r_N
    let mut rs_owned: Vec<FrRepr> = vec![];
    let mut rs: Vec<&[u64; 4]> = vec![];
    for _ in 0..params.n {
        let r = random_scalar().into_repr();
        rs_owned.push(r);
    }
    for item in rs_owned.iter() {
        rs.push(&item.0);
    }

    // Compute:
    // S = prod_{i=1}^{N-1} ("g_1^{alpha^i}")^{r_i}
    // R_1 = prod_{i=1}^{N} ("g_1^{alpha^i}")^{r_i} = S * ("g_1^{alpha^N}")^{r_N}
    // R_2 = prod_{i=1}^{N} ("g_2^{alpha^i}")^{r_i}
    // T = prod{i=1}^{N-1} ("g_1^{alpha^{i+1}}")^{r_i}
    // U_1 = prod{i=1}^{N-1} ("g_1^{alpha^{i+N+1}")^{r_i}
    // U_2 = prod{i=1}^{N-1} ("g_2^{alpha^{i+N+1}")^{r_i}

    let pt_s: bls12_381::G1Affine = G1Affine::sum_of_products(
        &params.g1_alpha_1_to_n[0..params.n - 1],
        &rs[0..params.n - 1],
    )
    .into_affine();
    let pt_r1: bls12_381::G1Affine = {
        let mut tmp = params.g1_alpha_1_to_n[params.n - 1]
            .mul(Fr::from_repr(rs_owned[params.n - 1]).unwrap());
        tmp.add_assign_mixed(&pt_s);
        tmp.into_affine()
    };
    let pt_r2 = G2Affine::sum_of_products(&params.g2_alpha_1_to_n[0..params.n], &rs[0..params.n])
        .into_affine();
    let pt_t =
        G1Affine::sum_of_products(&params.g1_alpha_1_to_n[1..params.n], &rs[0..params.n - 1])
            .into_affine();
    let pt_u1 = G1Affine::sum_of_products(
        &params.g1_alpha_nplus2_to_2n[0..params.n - 1],
        &rs[0..params.n - 1],
    )
    .into_affine();
    let pt_u2 = G2Affine::sum_of_products(
        &params.g2_alpha_nplus2_to_2n[0..params.n - 1],
        &rs[0..params.n - 1],
    )
    .into_affine();

    let g2 = G2Affine::one();
    let g1 = G1Affine::one();
    let g2alpha = &params.g2_alpha_1_to_n[0];

    // Then check
    // 1: e(R_1, g_2) = e(g_1, R_2)
    // which essentially checks e("g_1^{alpha^i}", g_2) = e(g_1, "g_2^{alpha^i}") for all 1<=i<=N
    if g2.pairing_with(&pt_r1) != g1.pairing_with(&pt_r2) {
        return false;
    }

    // 2: e(S, g_2^alpha) = e(T, g_2)
    // which essentially checks e("g_1^{alpha^i}", g_2^alpha) = e(g_1^{alpha^{i+1}}, g_2) for all 1<=i<=N-1
    if pt_s.pairing_with(g2alpha) != pt_t.pairing_with(&g2) {
        return false;
    }

    // 3: e(g_1^{alpha^N}, g_2^alpha) = "e(g_1, g_2)^{alpha^{N+1}}"
    let mut tmp = params.g1_alpha_1_to_n[params.n - 1].pairing_with(g2alpha);
    tmp.sub_assign(&params.gt_alpha_nplus1);
    if !tmp.is_zero() {
        return false;
    }

    // 4: e(T, g_2^{alpha^N}) = e(U_1, g_2) = e(g_1, U_2)
    let tmp = pt_t.pairing_with(&params.g2_alpha_1_to_n[params.n - 1]);
    if tmp != pt_u1.pairing_with(&g2) || tmp != pt_u2.pairing_with(&g1) {
        return false;
    }

    true
}