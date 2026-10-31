//! Thin canonical-source consumer; learning/rank stay in the Graph facade.
use hymeko::{
    ir::{canonical_hash::build_path_index, incidence::build_incidence_index},
    module_store::{execution::CompiledModel, source_provider::StdFsProvider},
    util::real_parser::RealParser,
};
use hymeko_graph::{
    MatroidRef,
    matroid::{
        BinaryBasis, BinaryMatroidWorkspace, BinaryReconstructionOptions, binary_frontier_barrier,
        binary_frontier_search, binary_latent_proposals, binary_reconstruction_search,
        binary_weighted_basis,
    },
};
use std::{env, path::Path};

fn column_weights(text: &str, count: usize) -> Result<Vec<f64>, Box<dyn std::error::Error>> {
    let values: Vec<f64> = text
        .split_whitespace()
        .map(str::parse)
        .collect::<Result<_, _>>()?;
    if values.len() != count || values.iter().any(|x| !x.is_finite() || *x < 0.0) {
        return Err("Expected one finite nonnegative cost per canonical column".into());
    }
    Ok(values)
}

fn model_digest(model: &CompiledModel) -> String {
    model
        .semantic_digest()
        .0
        .0
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn basis_json(basis: &BinaryBasis) -> String {
    let ids: Vec<_> = basis.elements.iter().map(|x| x.raw()).collect();
    let coordinates: Vec<_> = basis
        .coordinates
        .iter()
        .map(|c| {
            format!(
                "{{\"arc\":{},\"support\":{:?}}}",
                c.arc.raw(),
                c.support.iter().map(|x| x.raw()).collect::<Vec<_>>()
            )
        })
        .collect();
    format!(
        "{{\"elements\":{ids:?},\"weight\":{},\"coordinates\":[{}]}}",
        basis.weight,
        coordinates.join(",")
    )
}

fn reconstruction_options(
    args: &mut Vec<String>,
) -> Result<BinaryReconstructionOptions, Box<dyn std::error::Error>> {
    let mut options = BinaryReconstructionOptions::default();
    let mut index = 0;
    while index < args.len() {
        if !args[index].starts_with("--") {
            index += 1;
            continue;
        }
        let flag = args.remove(index);
        if index == args.len() {
            return Err(format!("Missing value for {flag}").into());
        }
        let value: usize = args.remove(index).parse()?;
        match flag.as_str() {
            "--beam" => options.beam = value,
            "--exchange-budget" => options.max_exchanges = value,
            "--exchanges-per-state" => options.exchanges_per_state = value,
            "--state-budget" => options.max_states = value,
            "--extra-products" => options.max_extra_products = value,
            "--target-cost" => options.target_cost = Some(value),
            "--column-offset" => options.column_offset = value,
            _ => return Err(format!("Unknown reconstruction option {flag}").into()),
        }
    }
    Ok(options)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args: Vec<_> = env::args().skip(1).collect();
    if args.first().is_some_and(|a| a == "--frontier") {
        return run_frontier(&args[1..], FrontierAction::Search);
    }
    if args.first().is_some_and(|a| a == "--barrier") {
        return run_frontier(&args[1..], FrontierAction::Barrier);
    }
    let basis_only = args.first().is_some_and(|a| a == "--basis");
    if basis_only {
        args.remove(0);
    }
    let options = reconstruction_options(&mut args)?;
    if !(2..=4).contains(&args.len()) {
        return Err("usage: binary_matroid MODEL.hymeko EXACT_FORM_SCOPE [COLUMN_WEIGHTS.txt [FREE:TARGETS:REQUIRED_TAIL]]".into());
    }
    let model = CompiledModel::compile(StdFsProvider::new(), RealParser, Path::new(&args[0]))
        .map_err(|e| format!("Core compilation: {e:?}"))?;
    let ir = &model.program().ir;
    let mut ground = scoped_columns(&model, &args[1])?;
    if let Some(path) = args.get(2) {
        let count = ground.len();
        for (entry, weight) in ground
            .iter_mut()
            .zip(column_weights(&std::fs::read_to_string(path)?, count)?)
        {
            entry.1 = weight;
        }
    }
    let index = build_incidence_index(ir)?;
    let view = MatroidRef::new(&index);
    let mut workspace = BinaryMatroidWorkspace::default();
    if basis_only {
        if args.len() > 3 {
            return Err("--basis accepts only MODEL SCOPE [WEIGHTS]".into());
        }
        let basis = binary_weighted_basis(view, &ground, &mut workspace)?;
        println!(
            "{{\"core_semantic_digest\":\"{}\",\"direct_core_view\":true,\"rank\":{},\"baseline\":{}}}",
            model_digest(&model),
            basis.elements.len(),
            basis_json(&basis)
        );
        return Ok(());
    }
    if let Some(config) = args.get(3) {
        let counts: Vec<usize> = config
            .split(':')
            .map(str::parse)
            .collect::<Result<_, _>>()?;
        if counts.len() != 3
            || counts[0] + counts[1] > ground.len()
            || counts[2] == 0
            || counts[2] > counts[1]
        {
            return Err("Invalid reconstruction column boundaries".into());
        }
        let ids: Vec<_> = ground.iter().map(|x| x.0).collect();
        let result = binary_reconstruction_search(
            view,
            &ground,
            &ids[..counts[0]],
            &ids[counts[0]..ids.len() - counts[1]],
            &ids[ids.len() - counts[2]..],
            &mut workspace,
            options,
        )?;
        let digest = model_digest(&model);
        let candidates: Vec<_> = result.candidates.iter().map(basis_json).collect();
        println!(
            concat!(
                "{{\"core_semantic_digest\":\"{}\",\"direct_core_view\":true,",
                "\"rank\":{},\"baseline\":{},\"candidates\":[{}],",
                "\"exchanges_evaluated\":{},\"budget_reached\":{},",
                "\"free\":{:?},\"required\":{:?},\"states_visited\":{},",
                "\"state_budget_reached\":{},\"target_reached\":{},\"maximum_paid_columns\":{},\"neighborhood_capped\":{},",
                "\"lower_bound\":{},\"relaxation_target\":{},\"quotient_classes\":{},",
                "\"target_infeasible_certified\":{},\"proof_scope\":\"fixed_free_prefix_available_columns\"}}"
            ),
            digest,
            result.baseline.elements.len(),
            basis_json(&result.baseline),
            candidates.join(","),
            result.exchanges_evaluated,
            result.budget_reached || result.state_budget_reached || result.neighborhood_capped,
            ids[..counts[0]].iter().map(|a| a.raw()).collect::<Vec<_>>(),
            ids[ids.len() - counts[2]..]
                .iter()
                .map(|a| a.raw())
                .collect::<Vec<_>>(),
            result.states_visited,
            result.state_budget_reached,
            result.target_reached,
            result.maximum_paid_columns,
            result.neighborhood_capped,
            result.lower_bound,
            result.relaxation_target,
            result.quotient_classes,
            result.target_infeasible_certified,
        );
        return Ok(());
    }
    let report = binary_latent_proposals(view, &ground, &mut workspace, 16)?;
    let digest = model_digest(&model);
    let candidates: Vec<_> = report.candidates.iter().map(basis_json).collect();
    let descriptors: Vec<_> = report
        .descriptors
        .iter()
        .map(|(arc, x)| format!("{{\"arc\":{},\"point\":{x:?}}}", arc.raw()))
        .collect();
    println!(
        "{{\"core_semantic_digest\":\"{digest}\",\"direct_core_view\":true,\"rank\":{},\"epochs\":{},\"prototypes\":{:?},\"baseline\":{},\"candidates\":[{}],\"descriptors\":[{}]}}",
        report.baseline.elements.len(),
        report.epochs,
        report.prototypes,
        basis_json(&report.baseline),
        candidates.join(","),
        descriptors.join(",")
    );
    Ok(())
}

fn scoped_columns(
    model: &CompiledModel,
    scope: &str,
) -> Result<Vec<(hymeko::common::ids::HyperArcId, f64)>, Box<dyn std::error::Error>> {
    let ir = &model.program().ir;
    let paths = build_path_index(ir)?;
    let symbols: Option<Vec<_>> = scope
        .split('.')
        .map(|s| model.interner().get_id(s))
        .collect();
    let root = symbols
        .and_then(|s| paths.decl_of(&s))
        .ok_or("Missing exact form scope")?;
    let mut ground = vec![];
    for child in ir.decl_children(root) {
        let (_, edge) = ir
            .edge_for_decl(child)
            .ok_or("Form scope contains a non-edge declaration")?;
        if edge.arcs.len() != 1 {
            return Err("Each form requires exactly one incidence hyperarc".into());
        }
        let arc = edge.arcs[0];
        ground.push((arc, ir.arcs[arc.raw()].refs.len() as f64));
    }
    Ok(ground)
}

enum FrontierAction {
    Search,
    Barrier,
}

fn run_frontier(args: &[String], action: FrontierAction) -> Result<(), Box<dyn std::error::Error>> {
    let valid = match action {
        FrontierAction::Search => args.len() == 4 || args.len() == 6,
        FrontierAction::Barrier => args.len() == 5,
    };
    if !valid {
        return Err("--frontier MODEL INPUT_SCOPE OUTPUT_SCOPE PRODUCT_SCOPE [BEAM BUDGET]".into());
    }
    let model = CompiledModel::compile(StdFsProvider::new(), RealParser, Path::new(&args[0]))
        .map_err(|e| format!("Core compilation: {e:?}"))?;
    let input = scoped_columns(&model, &args[1])?;
    let output = scoped_columns(&model, &args[2])?;
    let products = scoped_columns(&model, &args[3])?;
    if products.len() != input.len() {
        return Err("One function column per input/output pair required".into());
    }
    let index = build_incidence_index(&model.program().ir)?;
    let view = MatroidRef::new(&index);
    let mut ws = BinaryMatroidWorkspace::default();
    let product_basis = binary_weighted_basis(view, &products, &mut ws)?;
    if matches!(action, FrontierAction::Barrier) {
        let size: usize = args[4].parse()?;
        if size == 0 || input.len() % size != 0 {
            return Err("Group size must partition all terms".into());
        }
        let groups: Vec<_> = (0..input.len())
            .collect::<Vec<_>>()
            .chunks(size)
            .map(|g| g.to_vec())
            .collect();
        let proof = binary_frontier_barrier(
            view,
            &input.iter().map(|x| x.0).collect::<Vec<_>>(),
            &output.iter().map(|x| x.0).collect::<Vec<_>>(),
            &groups,
            &mut ws,
        )?;
        let digest = model_digest(&model);
        println!(
            "{{\"core_semantic_digest\":\"{digest}\",\"terms\":{},\"product_rank\":{},\"certified\":{},\"input_rank\":{},\"output_rank\":{},\"checks\":{},\"groups\":{},\"input_cover\":{},\"active_groups\":{},\"scope\":\"fixed_tensor_separate_linear_banks\"}}",
            input.len(),
            product_basis.elements.len(),
            proof.certified,
            proof.input_rank,
            proof.output_rank,
            proof.checks,
            proof.groups,
            proof.input_cover,
            proof.active_groups
        );
        return Ok(());
    }
    let beam = if args.len() == 6 { args[4].parse()? } else { 1 };
    let budget = if args.len() == 6 {
        args[5].parse()?
    } else {
        10000
    };
    let result = binary_frontier_search(
        view,
        &input.iter().map(|x| x.0).collect::<Vec<_>>(),
        &output.iter().map(|x| x.0).collect::<Vec<_>>(),
        &mut ws,
        beam,
        budget,
    )?;
    let steps: Vec<_> = result
        .steps
        .iter()
        .map(|s| {
            format!(
                "{{\"term\":{},\"input_rank_after\":{},\"input_before\":{},\"output_after\":{}}}",
                s.term,
                s.input_rank_after,
                basis_json(&s.input_before),
                basis_json(&s.output_after)
            )
        })
        .collect();
    let digest = model_digest(&model);
    println!(
        "{{\"core_semantic_digest\":\"{digest}\",\"terms\":{},\"product_rank\":{},\"baseline_peak\":{},\"peak\":{},\"lower_bound\":{},\"evaluated\":{},\"beam_truncated\":{},\"budget_reached\":{},\"steps\":[{}]}}",
        input.len(),
        product_basis.elements.len(),
        result.baseline_peak,
        result.peak,
        result.lower_bound,
        result.evaluated,
        result.beam_truncated,
        result.budget_reached,
        steps.join(",")
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::column_weights;
    #[test]
    fn explicit_column_costs_admit_free_prefix_and_reject_missing_or_invalid_costs() {
        assert_eq!(
            column_weights("0 0 1 1000", 4).unwrap(),
            vec![0., 0., 1., 1000.]
        );
        for text in ["0 1", "0 0 NaN 1", "0 0 inf 1", "0 0 -1 1"] {
            assert!(column_weights(text, 4).is_err());
        }
    }
}
