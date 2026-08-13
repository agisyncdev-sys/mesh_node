use halo2_proofs::{
    circuit::{Layouter, SimpleFloorPlanner, Value},
    plonk::{
        Advice, Circuit, Column, ConstraintSystem, Error, Instance, Selector,
        keygen_pk, keygen_vk, create_proof, verify_proof, ProvingKey, VerifyingKey, SingleVerifier
    },
    poly::commitment::Params,
    transcript::{
        Blake2bRead, Blake2bWrite, Challenge255, TranscriptRead, TranscriptWrite,
    },
    pasta::{EqAffine, Fp},
};
use ff::PrimeField;
use rand_core::OsRng;
use std::sync::OnceLock;

// Config and Chip
#[derive(Clone)]
pub struct MacConfig {
    w: Column<Advice>,
    x: Column<Advice>,
    b: Column<Advice>,
    y_adv: Column<Advice>,
    y: Column<Instance>,
    s_mac: Selector,
}

#[derive(Clone)]
pub struct MacCircuit {
    pub w: Value<Fp>,
    pub x: Value<Fp>,
    pub b: Value<Fp>,
}

impl Circuit<Fp> for MacCircuit {
    type Config = MacConfig;
    type FloorPlanner = SimpleFloorPlanner;

    fn without_witnesses(&self) -> Self {
        Self {
            w: Value::unknown(),
            x: Value::unknown(),
            b: Value::unknown(),
        }
    }

    fn configure(meta: &mut ConstraintSystem<Fp>) -> Self::Config {
        let w = meta.advice_column();
        let x = meta.advice_column();
        let b = meta.advice_column();
        let y_adv = meta.advice_column();
        let y = meta.instance_column();
        let s_mac = meta.selector();

        meta.enable_equality(w);
        meta.enable_equality(x);
        meta.enable_equality(b);
        meta.enable_equality(y_adv);
        meta.enable_equality(y);

        meta.create_gate("mac", |meta| {
            let w_val = meta.query_advice(w, halo2_proofs::poly::Rotation::cur());
            let x_val = meta.query_advice(x, halo2_proofs::poly::Rotation::cur());
            let b_val = meta.query_advice(b, halo2_proofs::poly::Rotation::cur());
            let y_val = meta.query_advice(y_adv, halo2_proofs::poly::Rotation::cur());
            let s_mac = meta.query_selector(s_mac);

            vec![s_mac * (w_val * x_val + b_val - y_val)]
        });

        MacConfig { w, x, b, y_adv, y, s_mac }
    }

    fn synthesize(&self, config: Self::Config, mut layouter: impl Layouter<Fp>) -> Result<(), Error> {
        let out_cell = layouter.assign_region(
            || "mac",
            |mut region| {
                config.s_mac.enable(&mut region, 0)?;
                region.assign_advice(|| "w", config.w, 0, || self.w)?;
                region.assign_advice(|| "x", config.x, 0, || self.x)?;
                region.assign_advice(|| "b", config.b, 0, || self.b)?;
                let y_val = self.w * self.x + self.b;
                let y_cell = region.assign_advice(|| "y", config.y_adv, 0, || y_val)?;
                Ok(y_cell.cell())
            },
        )?;
        
        layouter.constrain_instance(out_cell, config.y, 0)?;
        Ok(())
    }
}

static ZK_KEYS: OnceLock<(Params<EqAffine>, ProvingKey<EqAffine>, VerifyingKey<EqAffine>)> = OnceLock::new();

pub fn get_keys() -> &'static (Params<EqAffine>, ProvingKey<EqAffine>, VerifyingKey<EqAffine>) {
    ZK_KEYS.get_or_init(|| {
        println!("[Halo2] Generating IPA Setup on Pasta...");
        let params: Params<EqAffine> = Params::new(4); 
        
        let empty_circuit = MacCircuit {
            w: Value::unknown(),
            x: Value::unknown(),
            b: Value::unknown(),
        };
        
        let vk = keygen_vk(&params, &empty_circuit).unwrap();
        let pk = keygen_pk(&params, vk.clone(), &empty_circuit).unwrap();
        println!("[Halo2] Setup complete.");
        (params, pk, vk)
    })
}

pub fn generate_zk_proof(w_val: u64, x_val: u64, b_val: u64) -> (Vec<u8>, Vec<u8>) {
    let (params, pk, _) = get_keys();
    let mut rng = OsRng;
    
    let w = Fp::from(w_val);
    let x = Fp::from(x_val);
    let b = Fp::from(b_val);
    let y = Fp::from(w_val * x_val + b_val);
    
    let circuit = MacCircuit {
        w: Value::known(w),
        x: Value::known(x),
        b: Value::known(b),
    };
    
    let mut transcript = Blake2bWrite::<_, EqAffine, Challenge255<_>>::init(vec![]);
    
    create_proof(
        params,
        pk,
        &[circuit],
        &[&[&[y]]],
        &mut rng,
        &mut transcript,
    ).unwrap();
    
    let proof = transcript.finalize();
    (proof, y.to_repr().as_ref().to_vec())
}

pub fn verify_zk_payload(proof_bytes: &[u8], inputs_bytes: &[u8]) -> bool {
    let (params, _, vk) = get_keys();
    
    if inputs_bytes.len() != 32 {
        return false;
    }
    
    let mut repr = <Fp as PrimeField>::Repr::default();
    repr.as_mut().copy_from_slice(inputs_bytes);
    let c_opt = Fp::from_repr(repr);
    if c_opt.is_none().into() {
        return false;
    }
    let c = c_opt.unwrap();
    
    let strategy = SingleVerifier::new(params);
    let mut transcript = Blake2bRead::<_, _, Challenge255<_>>::init(proof_bytes);
    
    verify_proof(
        params,
        vk,
        strategy,
        &[&[&[c]]],
        &mut transcript,
    ).is_ok()
}
