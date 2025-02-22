#![cfg_attr(not(feature = "std"), no_std)]

use frame_support::{
    sp_runtime::traits::AtLeast32BitUnsigned,
    traits::{Currency, ReservableCurrency},
    BoundedVec,
};
use frame_system::Config as SystemConfig;
/// Edit this file to define custom logic or remove it if it is not needed.
/// Learn more about FRAME and the core library of Substrate FRAME pallets:
/// <https://docs.substrate.io/v3/runtime/frame>
pub use pallet::*;
use sp_std::prelude::*;

#[cfg(test)]
mod mock;

#[cfg(test)]
mod tests;

#[cfg(feature = "runtime-benchmarks")]
mod benchmarking;

mod elo_comp;

type DepositBalanceOf<T> =
    <<T as Config>::Currency as Currency<<T as SystemConfig>::AccountId>>::Balance;

#[frame_support::pallet]
pub mod pallet {
    use crate::elo_comp::EloRank;

    use super::*;
    use frame_support::pallet_prelude::*;
    use frame_system::pallet_prelude::*;
    use sp_core::H256;

    #[derive(Encode, Decode, Default, Clone, Debug, PartialEq, Eq, TypeInfo, MaxEncodedLen)]
    #[cfg_attr(feature = "std", derive(serde::Serialize, serde::Deserialize))]
    /// Holds the data which is associated to an auditor.
    /// # Fields
    ///
    /// * `score` - the Auditors Eloscore of type `Option<u32>`. This is also used to keep track of the auditor's approval status.
    ///             Unapproved auditors have a `score` of value `None`
    ///
    /// * `profile_hash` - A hash of the profile that the user submitted. Is supposed to be the hash of a markdown document which describes the user's
    ///                    background and qualification of being an auditor.
    ///
    /// * `approved_by` - A user needs three approval's from already approved auditor's.Therefore this `BoundedVec<AccountId, ConstU32<3>>` can hold up
    ///                   to three `AccountId`'s of approving auditors.
    ///
    pub struct AuditorData<Hash, AccountId> {
        pub score: Option<u32>,
        pub profile_hash: Hash,
        pub approved_by: BoundedVec<AccountId, ConstU32<3>>,
    }

    #[derive(Encode, Decode, Debug, Clone, PartialEq, Eq, TypeInfo, MaxEncodedLen)]
    /// Denotes the result/winner after challenging an auditor
    pub enum Winner {
        Player0,
        Player1,
        Draw,
    }

    #[pallet::config]
    /// The `qdao-audit-pallet` Config trait implementation
    pub trait Config: frame_system::Config {
        /// Because this pallet emits events, it depends on the runtime's definition of an event.
        type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;

        /// Units for balance
        type Balance: Member + Parameter + AtLeast32BitUnsigned + Default + Copy;

        /// Currency mechanism
        type Currency: ReservableCurrency<Self::AccountId>;

        #[pallet::constant]
        /// Minimum amount which is required for an Auditor to be able to sign up.
        type MinAuditorStake: Get<DepositBalanceOf<Self>>;

        #[pallet::constant]
        /// Initial score for an auditor which signed up and received 3 approvals
        type InitialAuditorScore: Get<u32>;

        #[pallet::constant]
        /// Minimal score which allows auditors to approve other auditors
        type MinimalApproverScore: Get<u32>;
    }

    #[pallet::pallet]
    #[pallet::generate_store(pub(super) trait Store)]
    pub struct Pallet<T>(_);

    #[pallet::storage]
    #[pallet::getter(fn auditor_score)]
    /// Storage for auditor scores
    /// If a new Auditor signed up whose approval is pending, the Auditor scrore will be None
    pub(super) type AuditorMap<T: Config> =
        StorageMap<_, Blake2_128Concat, T::AccountId, AuditorData<sp_core::H256, T::AccountId>>;

    type AuditorMapData<T> = (
        <T as frame_system::Config>::AccountId,
        AuditorData<sp_core::H256, <T as frame_system::Config>::AccountId>,
    );

    #[pallet::genesis_config]
    /// Allows a Genesis config with pre-assigned Auditors
    pub struct GenesisConfig<T: Config> {
        pub auditor_map: Vec<AuditorMapData<T>>,
    }

    #[cfg(feature = "std")]
    impl<T: Config> Default for GenesisConfig<T> {
        fn default() -> Self {
            Self {
                auditor_map: Default::default(),
            }
        }
    }

    #[pallet::genesis_build]
    impl<T: Config> GenesisBuild<T> for GenesisConfig<T> {
        fn build(&self) {
            // <SingleValue<T>>::put(&self.single_value);
            for (a, b) in &self.auditor_map {
                <AuditorMap<T>>::insert(a, b);
            }
        }
    }

    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    /// Events which are emitted by `qdao-audit-pallet`
    pub enum Event<T: Config> {
        /// New Auditor signed up
        SignedUp { who: T::AccountId },
        /// A new challenge result was submitted
        GameResult {
            player0: T::AccountId,
            player1: T::AccountId,
            winner: Winner,
        },
    }

    #[pallet::error]
    /// Errors of `qdao-audit-pallet`
    pub enum Error<T> {
        /// Error names should be descriptive.
        NoneValue,
        /// Errors should have helpful documentation associated with them.
        StorageOverflow,
        /// Auditor is already signed up
        AlreadySignedUp,
        /// Auditor doesn't provide enough stake for sign up
        InsufficientStake,
        /// User is not registered as an Auditor
        UnknownAuditor,
        /// User is registered as an Auditor but has not been approved
        UnapprovedAuditor,
        /// Auditor is registered, but the reputation score is to low for the intended interaction
        ReputationTooLow,
        /// The user that should eb approved is note registered as an Auditor
        UnknownApprovee,
        /// The approvee is already an Auditor
        AlreadyAuditor,
        /// The approvee already received an approval by the sender
        AlreadyApproved,
        /// Eloscore computational overflow (expected not to happen with Eloscore formula)
        UnexpectedEloOverflow,
    }

    #[pallet::call]
    impl<T: Config> Pallet<T> {
        #[pallet::weight(Weight::from_ref_time(10_000) + T::DbWeight::get().writes(1))]
        /// Signs up a new Auditor, the auditor has to provide a hashed version of his auditor profile
        /// # Arguments
        ///
        /// * `profile_hash` - a hash auf the new auditors profile of type `H256`
        ///
        pub fn sign_up(origin: OriginFor<T>, profile_hash: H256) -> DispatchResult {
            // Check that the extrinsic was signed and get the signer.
            // This function will return an error if the extrinsic is not signed.
            // https://docs.substrate.io/v3/runtime/origins
            let sender = ensure_signed(origin)?;

            // Ensure that auditor is not already signed up
            ensure!(
                !AuditorMap::<T>::contains_key(&sender),
                Error::<T>::AlreadySignedUp
            );

            T::Currency::reserve(&sender, T::MinAuditorStake::get())?;

            // Register new Auditor
            let auditor_data = AuditorData::<H256, T::AccountId> {
                score: None,
                profile_hash,
                approved_by: BoundedVec::with_bounded_capacity(3),
            };
            <AuditorMap<T>>::insert(sender.clone(), auditor_data);

            // Emit an event.
            Self::deposit_event(Event::SignedUp { who: sender });
            // Return a successful DispatchResultWithPostInfo
            Ok(())
        }

        #[pallet::weight(Weight::from_ref_time(10_000) + T::DbWeight::get().writes(1))]
        /// An Auditor which signed up for auditor status can call this extrinsic to update their profile hash
        /// # Arguments
        ///
        /// * `profile_hash` - a hash auf the new auditors profile of type `H256`
        ///
        pub fn update_profile(origin: OriginFor<T>, profile_hash: H256) -> DispatchResult {
            let sender = ensure_signed(origin)?;

            let mut auditor_data_to_update =
                <AuditorMap<T>>::try_get(&sender).map_err(|_| Error::<T>::UnknownAuditor)?;

            auditor_data_to_update.profile_hash = profile_hash;

            <AuditorMap<T>>::insert(sender, auditor_data_to_update);

            Ok(())
        }

        #[pallet::weight(Weight::from_ref_time(10_000) + T::DbWeight::get().writes(1))]

        /// Is called by an auditor which signed up for auditor status to cancel their
        /// account and to unreserve the associated funds.
        ///
        pub fn cancel_account(origin: OriginFor<T>) -> DispatchResult {
            let sender = ensure_signed(origin)?;

            ensure!(
                <AuditorMap<T>>::contains_key(&sender),
                Error::<T>::UnknownAuditor
            );

            T::Currency::unreserve(&sender, T::MinAuditorStake::get());

            <AuditorMap<T>>::remove(sender);

            Ok(())
        }

        #[pallet::weight(Weight::from_ref_time(10_000) + T::DbWeight::get().writes(1))]
        /// Is called to give an approval to someone who signed up as an auditor. Can only be called by user's which are already approved auditors.
        /// # Arguments
        ///
        /// * `to_approve` - the account id of type `T::AccountId` of the user which should get approved.
        ///
        pub fn approve_auditor(origin: OriginFor<T>, to_approve: T::AccountId) -> DispatchResult {
            let sender = ensure_signed(origin)?;

            // Get sender data and check that sender is qualified to approve auditors
            let sender_data =
                <AuditorMap<T>>::try_get(&sender).map_err(|_| Error::<T>::UnknownAuditor)?;
            let sender_score = sender_data.score.ok_or(Error::<T>::UnapprovedAuditor)?;
            ensure!(
                sender_score >= T::MinimalApproverScore::get(),
                Error::<T>::ReputationTooLow
            );

            // Get data of user which should get approved
            let mut to_approve_data =
                <AuditorMap<T>>::try_get(&to_approve).map_err(|_| Error::<T>::UnknownApprovee)?;

            // Make sure that has not already auditor status
            ensure!(to_approve_data.score.is_none(), Error::<T>::AlreadyAuditor);

            // Make sure that user was not already approved by sender
            ensure!(
                !to_approve_data.approved_by.contains(&sender),
                Error::<T>::AlreadyApproved,
            );

            // Add approval by sender
            to_approve_data
                .approved_by
                .try_push(sender)
                .map_err(|_| Error::<T>::StorageOverflow)?;

            // If user has 3 approvals, give user Auditor status
            if to_approve_data.approved_by.len() == 3 {
                to_approve_data.score = Some(T::InitialAuditorScore::get());
            }

            // Update user data
            <AuditorMap<T>>::insert(to_approve, to_approve_data);

            Ok(())
        }

        #[pallet::weight(Weight::from_ref_time(10_000) + T::DbWeight::get().writes(1))]
        /// Returns a person with the name given them
        ///
        /// # Arguments
        ///
        /// * `player0` - AccountId of player 0
        ///
        /// * `player1` - AccountId of player 1
        ///

        pub fn game_result(
            origin: OriginFor<T>,
            player0: T::AccountId,
            player1: T::AccountId,
            winner: Winner,
        ) -> DispatchResult {
            ensure_root(origin)?;

            <Self as Game<_>>::apply_result(player0, player1, winner)?;

            Ok(())
        }
    }

    pub trait Game<T: frame_system::Config> {
        /// Is called after a auditor was challenged to transmit the result of the challenge
        ///
        /// * `player0` - ``T::AccountId`` of player 0
        ///
        /// * `player1` - ``T::AccountId`` of player 1
        ///
        /// * `winner` - ``Winner`` the enum that indicates who won the challenge
        ///
        fn apply_result(
            player0: T::AccountId,
            player1: T::AccountId,
            winner: Winner,
        ) -> DispatchResult;
    }

    impl<T: Config> Game<T> for Pallet<T> {
        /// Is called after a auditor was challenged to transmit the result of the challenge. This finally updates the Eloscores
        /// of both players accordingly.
        ///
        /// * `player0` - ``T::AccountId`` of player 0
        ///
        /// * `player1` - ``T::AccountId`` of player 1
        ///
        /// * `winner` - ``Winner`` the enum that indicates who won the challenge
        ///

        fn apply_result(
            player0: T::AccountId,
            player1: T::AccountId,
            winner: Winner,
        ) -> DispatchResult {
            // Get data and particularly scores of both players
            let mut player0_data =
                <AuditorMap<T>>::try_get(&player0).map_err(|_| Error::<T>::UnknownAuditor)?;
            let player0_score = player0_data.score.ok_or(Error::<T>::UnapprovedAuditor)?;
            let mut player1_data =
                <AuditorMap<T>>::try_get(&player1).map_err(|_| Error::<T>::UnknownAuditor)?;
            let player1_score = player1_data.score.ok_or(Error::<T>::UnapprovedAuditor)?;

            // Map winner and looser scores accordingly
            let (winner_score, looser_score) = match winner {
                Winner::Player0 => (player0_score, player1_score),
                Winner::Player1 => (player1_score, player0_score),
                _ => return Ok(()),
            };

            // Instantiate EloRank, compute new scores
            let elo = EloRank { k: 32 };
            let (winner_new, looser_new) = elo
                .calculate(winner_score, looser_score)
                .map_err(|_| Error::<T>::UnexpectedEloOverflow)?;

            // Map score results accordingly
            (player0_data.score, player1_data.score) = match winner {
                Winner::Player0 => (Some(winner_new), Some(looser_new)),
                Winner::Player1 => (Some(looser_new), Some(winner_new)),
                _ => return Ok(()),
            };

            // Write update of player data to runtime storage
            <AuditorMap<T>>::insert(&player0, player0_data);
            <AuditorMap<T>>::insert(&player1, player1_data);

            // Emit GameResult event
            Self::deposit_event(Event::GameResult {
                player0,
                player1,
                winner,
            });

            Ok(())
        }
    }
}
