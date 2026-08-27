//! RTL function storage and checked construction.

extern crate alloc;

use alloc::format;
use alloc::string::String;

use crate::{BlockId, Cfg, EdgeId};

use super::dialect::Dialect;
use super::error::{Error, Result};
use super::expr::Expr;
use super::statement::{Statement, StatementNode};
use super::types::ValueShape;

/// One RTL function backed by a cfglib control-flow graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Function<D: Dialect> {
    pub(super) cfg: Cfg<StatementNode<D>, D::Edge>,
    pub(super) source: D::Source,
}

impl<D: Dialect> Function<D> {
    /// Returns the exact edge-bearing RTL control-flow graph.
    #[must_use]
    pub const fn cfg(&self) -> &Cfg<StatementNode<D>, D::Edge> {
        &self.cfg
    }

    /// Returns the source function identity.
    #[must_use]
    pub const fn source(&self) -> &D::Source {
        &self.source
    }
}

/// Incremental checked builder of one RTL function.
pub struct FunctionBuilder<D: Dialect> {
    cfg: Cfg<StatementNode<D>, D::Edge>,
    source: D::Source,
}

impl<D: Dialect> FunctionBuilder<D> {
    /// Creates a builder with an empty synthetic root block.
    #[must_use]
    pub fn new(source: D::Source) -> Self {
        let mut cfg = Cfg::with_edge_payload();
        cfg.block_mut(cfg.entry()).set_label("root");
        Self { cfg, source }
    }

    /// Returns the synthetic root block.
    #[must_use]
    pub fn entry(&self) -> BlockId {
        self.cfg.entry()
    }

    /// Allocates a semantic block with a diagnostic label.
    pub fn new_block(&mut self, label: impl Into<String>) -> BlockId {
        let block = self.cfg.new_block();
        self.cfg.block_mut(block).set_label(label);
        block
    }

    /// Adds one exact semantic edge.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid endpoint.
    pub fn add_edge(
        &mut self,
        source: BlockId,
        target: BlockId,
        metadata: D::Edge,
    ) -> Result<EdgeId> {
        self.require_block(source)?;
        self.require_block(target)?;
        let kind = D::edge_kind(&metadata);
        Ok(self
            .cfg
            .add_edge_with_payload(source, target, kind, metadata))
    }

    /// Appends one statement to a block, validating its shapes.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid block, a width mismatch between a
    /// place and its value, repeated destination lanes, a non-scalar
    /// branch condition, or a width-changing reinterpretation.
    pub fn append(
        &mut self,
        block: BlockId,
        statement: Statement<D>,
        span: Option<D::SourceSpan>,
    ) -> Result<()> {
        self.require_block(block)?;
        validate_statement(&statement)?;
        self.cfg
            .block_mut(block)
            .push(StatementNode::new(statement, span));
        Ok(())
    }

    /// Completes the function.
    ///
    /// # Errors
    ///
    /// Returns an error when a control-flow statement sits before the
    /// end of its block.
    pub fn finish(self) -> Result<Function<D>> {
        for block in self.cfg.blocks() {
            let statements = block.instructions();
            for (index, node) in statements.iter().enumerate() {
                let terminator = matches!(
                    node.statement(),
                    Statement::Branch { .. } | Statement::Return { .. }
                );
                if terminator && index + 1 != statements.len() {
                    return Err(Error::InvalidConstruction(format!(
                        "control-flow statement is not last in block {}",
                        block.id()
                    )));
                }
            }
        }
        Ok(Function {
            cfg: self.cfg,
            source: self.source,
        })
    }

    fn require_block(&self, block: BlockId) -> Result<()> {
        if block.index() < self.cfg.block_count() {
            Ok(())
        } else {
            Err(Error::InvalidConstruction(format!(
                "block {block} is outside a {}-block function",
                self.cfg.block_count()
            )))
        }
    }
}

fn validate_statement<D: Dialect>(statement: &Statement<D>) -> Result<()> {
    match statement {
        Statement::Transfer { assignments, .. } => {
            for (place, value) in assignments {
                if place.lanes.is_empty() {
                    return Err(Error::InvalidConstruction(
                        "assignment writes no lanes".into(),
                    ));
                }
                let mut seen = [false; 256];
                for &lane in &place.lanes {
                    if core::mem::replace(&mut seen[usize::from(lane)], true) {
                        return Err(Error::InvalidConstruction(format!(
                            "destination repeats lane {lane}"
                        )));
                    }
                }
                let width = value.shape().lanes;
                if usize::from(width) != place.lanes.len() {
                    return Err(Error::InvalidConstruction(format!(
                        "value width {width} does not match {}-lane destination",
                        place.lanes.len()
                    )));
                }
                validate_expr(value)?;
            }
            Ok(())
        }
        Statement::Effect { operands, .. } => {
            for operand in operands {
                validate_expr(operand)?;
            }
            Ok(())
        }
        Statement::Branch { condition } => {
            if condition.shape().lanes != 1 {
                return Err(Error::InvalidConstruction(
                    "branch condition is not scalar".into(),
                ));
            }
            validate_expr(condition)
        }
        Statement::Return { values } => {
            for value in values {
                validate_expr(value)?;
            }
            Ok(())
        }
    }
}

fn validate_expr<D: Dialect>(expr: &Expr<D>) -> Result<()> {
    match expr {
        Expr::Read { lanes, .. } => {
            if lanes.is_empty() {
                return Err(Error::InvalidConstruction("read selects no lanes".into()));
            }
            Ok(())
        }
        Expr::Const { bits, shape } => {
            if bits.len() != usize::from(shape.lanes) {
                return Err(Error::InvalidConstruction(format!(
                    "constant carries {} lanes for a {}-lane shape",
                    bits.len(),
                    shape.lanes
                )));
            }
            Ok(())
        }
        Expr::Apply { operands, .. } => {
            for operand in operands {
                validate_expr(operand)?;
            }
            Ok(())
        }
        Expr::Reinterpret { operand, shape } => {
            let ValueShape { lanes, .. } = operand.shape();
            if lanes != shape.lanes {
                return Err(Error::InvalidConstruction(format!(
                    "reinterpretation changes width {lanes} to {}",
                    shape.lanes
                )));
            }
            validate_expr(operand)
        }
    }
}
