// SPDX-License-Identifier: Apache-2.0

import Foundation

// The tests of the guard start this program again, as the guard and as the app (GuardTests.swift).
if CommandLine.arguments.contains("--guard-child") { exit(guardChildMain()) }
if CommandLine.arguments.contains("--guard-parent") { exit(guardParentMain()) }

// A write to a guard that is gone is an error for a test to look at, and not the end of the tests.
signal(SIGPIPE, SIG_IGN)

var tests: [(String, () -> Void)] = [
  ("registration gives the Rust side a whole table", testRegistrationGivesTheRustSideAWholeTable),
  (
    "a registration that fails returns its code and lets go of the platform",
    testARegistrationThatFailsReturnsItsCodeAndLetsGoOfThePlatform
  ),
  (
    "a registration that works keeps the platform for the life of the process",
    testARegistrationThatWorksKeepsThePlatformForTheLifeOfTheProcess
  ),
  (
    "limits are what the platform says and the room is checked",
    testLimitsAreWhatThePlatformSaysAndTheRoomIsChecked
  ),
  (
    "an open makes a screen and gives its pointer and number",
    testAnOpenMakesAScreenAndGivesItsPointerAndNumber
  ),
  (
    "opens that overlap each other and the calls of a screen are safe",
    testOpensThatOverlapEachOtherAndTheCallsOfAScreenAreSafe
  ),
  (
    "an open that fails writes the sentence and makes no pointer",
    testAnOpenThatFailsWritesTheSentenceAndMakesNoPointer
  ),
  (
    "an open that fails in another way has a sentence of its own",
    testAnOpenThatFailsInAnotherWayHasASentenceOfItsOwn
  ),
  (
    "a sentence that does not fit is cut between two characters",
    testASentenceThatDoesNotFitIsCutBetweenTwoCharacters
  ),
  (
    "an open with structs that are too short or missing is refused",
    testAnOpenWithStructsThatAreTooShortOrMissingIsRefused
  ),
  ("a message with no room writes nothing", testAMessageWithNoRoomWritesNothing),
  ("a frame reaches the screen as the Rust side built it", testAFrameReachesTheScreenAsTheRustSideBuiltIt),
  (
    "a frame that is not well formed never reaches the screen",
    testAFrameThatIsNotWellFormedNeverReachesTheScreen
  ),
  ("what the source says reaches the screen", testWhatTheSourceSaysReachesTheScreen),
  ("show, state and ended reach the screen", testShowStateAndEndedReachTheScreen),
  (
    "release tells the screen once and lets go of it",
    testReleaseTellsTheScreenOnceAndLetsGoOfIt
  ),
  (
    "a picture is shown when both the decoder and the Rust side have said so",
    testAPictureIsShownWhenBothTheDecoderAndTheRustSideHaveSaidSo
  ),
  (
    "an older picture is never shown after a newer one",
    testAnOlderPictureIsNeverShownAfterANewerOne
  ),
  (
    "a frame that never came out does not hold back the next",
    testAFrameThatNeverCameOutDoesNotHoldBackTheNext
  ),
  (
    "pictures that nobody asks for are kept up to the capacity",
    testPicturesThatNobodyAsksForAreKeptUpToTheCapacity
  ),
  (
    "what is older than the picture shown is let go of",
    testWhatIsOlderThanThePictureShownIsLetGoOf
  ),
  ("clear forgets everything", testClearForgetsEverythingIncludingWhatWasShown),
  (
    "the time a frame takes is smoothed and never out of range",
    testTheTimeAFrameTakesIsSmoothedAndNeverOutOfRange
  ),
  (
    "a key frame prepares the decoder and later frames do not",
    testAKeyFramePreparesTheDecoderAndLaterFramesDoNot
  ),
  ("new parameter sets prepare the decoder again", testNewParameterSetsPrepareTheDecoderAgain),
  (
    "a frame is not decoded before a key frame has come",
    testAFrameIsNotDecodedBeforeAKeyFrameHasCome
  ),
  (
    "a key frame that came with no parameter sets is refused",
    testAKeyFrameThatCameWithNoParameterSetsIsRefused
  ),
  (
    "parameter sets that the platform cannot use refuse the frame",
    testParameterSetsThatThePlatformCannotUseRefuseTheFrame
  ),
  (
    "after parameters that were refused nothing is decoded until a key frame comes",
    testAfterParametersThatWereRefusedNothingIsDecodedUntilAKeyFrameComes
  ),
  (
    "a frame that the decoder does not take makes the screen wait for a key frame",
    testAFrameThatTheDecoderDoesNotTakeMakesTheScreenWaitForAKeyFrame
  ),
  (
    "a picture is shown in either order of the decoder and the Rust side",
    testAPictureIsShownInEitherOrderOfTheDecoderAndTheRustSide
  ),
  (
    "a late failure is reported once and a key frame starts again",
    testALateFailureIsReportedOnceAndAKeyFrameStartsAgain
  ),
  (
    "a failure is not reported from the thread that is inside the call",
    testAFailureIsNotReportedFromTheThreadThatIsInsideTheCall
  ),
  (
    "nothing is shown or reported after the session ended",
    testNothingIsShownOrReportedAfterTheSessionEnded
  ),
  (
    "a picture on its way to the screen is there before the end takes it away",
    testAPictureOnItsWayToTheScreenIsThereBeforeTheEndTakesItAway
  ),
  (
    "pictures reach the screen one at a time in the order they were chosen",
    testPicturesReachTheScreenOneAtATimeInTheOrderTheyWereChosen
  ),
  (
    "a picture that comes after the end is not kept",
    testAPictureThatComesAfterTheEndIsNotKept
  ),
  (
    "releasing a screen ends it and lets go of what it held once",
    testReleasingAScreenEndsItAndLetsGoOfWhatItHeldOnce
  ),
  (
    "the state tells how long frames take and how hot the device is",
    testTheStateTellsHowLongFramesTakeAndHowHotTheDeviceIs
  ),
  ("a frame is given the sample and nothing of the sets", testAFrameIsGivenTheSampleAndNothingOfTheSets),
  (
    "parameter sets are copies split at the sequence sets",
    testParameterSetsAreCopiesSplitAtTheSequenceSets
  ),
  (
    "a failure that comes while a frame is prepared for is not undone",
    testAFailureThatComesWhileAFrameIsPreparedForIsNotUndone
  ),
  (
    "a screen that ends while a frame is prepared for decodes nothing",
    testAScreenThatEndsWhileAFrameIsPreparedForDecodesNothing
  ),
  (
    "a picture that was held is not shown after the end",
    testAPictureThatWasHeldIsNotShownAfterTheEnd
  ),
  ("only the key flag makes a key frame", testOnlyTheKeyFlagMakesAKeyFrame),
  (
    "the state can be read while pictures come out",
    testTheStateCanBeReadWhilePicturesComeOut
  ),
  (
    "places are made when asked, and only the missing ones",
    testPlacesAreMadeWhenAskedAndOnlyTheMissingOnesAre
  ),
  (
    "a place that cannot be made stops the filling and is tried again",
    testAPlaceThatCannotBeMadeStopsTheFillingAndIsTriedAgain
  ),
  ("a place is used by one window at a time", testAPlaceIsUsedByOneWindowAtATime),
  ("no place is taken before any is made", testNoPlaceIsTakenBeforeAnyIsMade),
  ("a count below one wants nothing", testACountBelowOneWantsNothing),
  (
    "fills at the same time make each place once",
    testFillsAtTheSameTimeMakeEachPlaceOnce
  ),
  (
    "a place can be taken while the others are still being made",
    testAPlaceCanBeTakenWhileTheOthersAreStillBeingMade
  ),
  (
    "places are taken and given from many threads without losing one",
    testPlacesAreTakenAndGivenFromManyThreadsWithoutLosingOne
  ),
  ("an attempt that works is not repeated", testAnAttemptThatWorksIsNotRepeated),
  (
    "an attempt that does not work is repeated after each delay and then given up",
    testAnAttemptThatDoesNotWorkIsRepeatedAfterEachDelayAndThenGivenUp
  ),
  ("trying stops as soon as an attempt works", testTryingStopsAsSoonAsAnAttemptWorks),
  ("no delays means one attempt", testNoDelaysMeansOneAttempt),
  (
    "places are made once the engine runs, as on a Mac",
    testPlacesAreMadeOnceTheEngineRunsAsTheMacDoes
  ),
  (
    "places that never come are given up on, and made when a window asks",
    testPlacesThatNeverComeAreGivenUpOnAndAskedForAgainWhenAWindowComes
  ),
  (
    "the sharer registration gives the Rust side a whole table",
    testTheSharerRegistrationGivesTheRustSideAWholeTable
  ),
  (
    "a sharer registration that fails returns its code and lets go of the platform",
    testASharerRegistrationThatFailsReturnsItsCodeAndLetsGoOfThePlatform
  ),
  (
    "a sharer registration that works keeps the platform for the life of the process",
    testASharerRegistrationThatWorksKeepsThePlatformForTheLifeOfTheProcess
  ),
  (
    "whether a window can be controlled is what the platform says each time",
    testWhetherAWindowCanBeControlledIsWhatThePlatformSaysEachTime
  ),
  (
    "a pick is shown and a taken down pick is told",
    testAPickIsShownAndATakenDownPickIsTold
  ),
  (
    "a pick that cannot be shown writes the sentence and anything else has one of its own",
    testAPickThatCannotBeShownWritesTheSentenceAndAnythingElseHasOneOfItsOwn
  ),
  (
    "the answer to a pick gives the window and what is known of it",
    testTheAnswerToAPickGivesTheWindowAndWhatIsKnownOfIt
  ),
  (
    "a display is given as a display and a window is not",
    testADisplayIsGivenAsADisplayAndAWindowIsNot
  ),
  (
    "a window and a display are called what they are in a sentence",
    testAWindowAndADisplayAreCalledWhatTheyAreInASentence
  ),
  (
    "a window with no title and no name is given with empty text",
    testAWindowWithNoTitleAndNoNameIsGivenWithEmptyText
  ),
  (
    "a title that is not ascii goes as it is",
    testATitleThatIsNotAsciiGoesAsItIs
  ),
  (
    "an answer that the Rust side refuses leaves the window with the platform",
    testAnAnswerThatTheRustSideRefusesLeavesTheWindowWithThePlatform
  ),
  (
    "a picker that failed and a person who gave up are told",
    testAPickerThatFailedAndAPersonWhoGaveUpAreTold
  ),
  (
    "the table has a screen call only when the platform can give the screen",
    testTheTableHasAScreenCallOnlyWhenThePlatformCanGiveTheScreen
  ),
  (
    "the table has a lift only when the platform can give the screen, and is whole",
    testTheTableHasALiftOnlyWhenThePlatformCanGiveTheScreenAndTheTableIsWhole
  ),
  (
    "a platform that says nothing of the screen cannot give it and has no entry for it",
    testAPlatformThatSaysNothingOfTheScreenCannotGiveItAndHasNoEntryForIt
  ),
  (
    "the screen is given as a display that the Rust side owns and never named",
    testTheScreenIsGivenAsADisplayThatTheRustSideOwnsAndNeverNamed
  ),
  (
    "the kind of the screen is passed on as the platform says and no name is written",
    testTheKindOfTheScreenIsPassedOnAsThePlatformSaysAndNoNameIsWritten
  ),
  (
    "a screen that cannot be given is told by its code and nothing is written",
    testAScreenThatCannotBeGivenIsToldByItsCodeAndNothingIsWritten
  ),
  (
    "a screen is asked for private only with the flag and an unknown flag fails without asking",
    testAScreenIsAskedForPrivateOnlyWithTheFlagAndAFlagThatIsNotKnownFailsWithoutAsking
  ),
  (
    "a screen call with structs that are too short or missing is refused without asking the platform",
    testAScreenCallWithStructsThatAreTooShortOrMissingIsRefusedWithoutAskingThePlatform
  ),
  (
    "what is lifted reaches the window as the two parts it is made of and no other part",
    testWhatIsLiftedReachesTheWindowAsTheTwoPartsItIsMadeOfAndNoOtherPart
  ),
  (
    "the table has an entry for the sound only when the platform can send it",
    testTheTableHasAnEntryForTheSoundOnlyWhenThePlatformCanSendIt
  ),
  (
    "what is heard reaches the window as yes or no and any other number is no",
    testWhatIsHeardReachesTheWindowAsYesOrNoAndAnyOtherNumberIsNo
  ),
  (
    "a window is prepared within what the other device takes",
    testAWindowIsPreparedWithinWhatTheOtherDeviceTakes
  ),
  (
    "a picture that cannot be made is a failure and nothing is written",
    testAPictureThatCannotBeMadeIsAFailureAndNothingIsWritten
  ),
  (
    "a prepare with structs that are too short or missing is refused",
    testAPrepareWithStructsThatAreTooShortOrMissingIsRefused
  ),
  (
    "a start gives the window its link and a failure writes its sentence",
    testAStartGivesTheWindowItsLinkAndAFailureWritesItsSentence
  ),
  (
    "the sentence of a failed start is cut between two characters where it does not fit",
    testTheSentenceOfAFailedStartIsCutBetweenTwoCharactersWhereItDoesNotFit
  ),
  (
    "the end and the release of a window reach it",
    testTheEndAndTheReleaseOfAWindowReachIt
  ),
  (
    "a window that was never started is released without an end",
    testAWindowThatWasNeverStartedIsReleasedWithoutAnEnd
  ),
  (
    "the picture is the window itself when it fits",
    testThePictureIsTheWindowItselfWhenItFits
  ),
  (
    "a large window is scaled down with its shape kept",
    testALargeWindowIsScaledDownWithItsShapeKept
  ),
  (
    "the picture stays within what the other device takes",
    testThePictureStaysWithinWhatTheOtherDeviceTakes
  ),
  (
    "the edges of the picture are even and never below two",
    testTheEdgesOfThePictureAreEvenAndNeverBelowTwo
  ),
  (
    "our own limits are within what the protocol allows",
    testOurOwnLimitsAreWithinWhatTheProtocolAllows
  ),
  (
    "no picture is made for a viewer that takes none",
    testNoPictureIsMadeForAViewerThatTakesNone
  ),
  (
    "the bitrate grows with the picture within bounds",
    testTheBitrateGrowsWithThePictureWithinBounds
  ),
  (
    "the clock and the questions are asked about the link",
    testTheClockAndTheQuestionsAreAskedAboutTheLink
  ),
  (
    "a clock and a question that are refused mean that the session is over",
    testAClockAndAQuestionThatAreRefusedMeanThatTheSessionIsOver
  ),
  (
    "what begin says is the kind of picture to make",
    testWhatBeginSaysIsTheKindOfPictureToMake
  ),
  (
    "a begin that is refused or not understood ends or gives up the picture",
    testABeginThatIsRefusedOrNotUnderstoodEndsOrGivesUpThePicture
  ),
  (
    "a begin that says yes without saying what kind makes no picture",
    testABeginThatSaysYesWithoutSayingWhatKindMakesNoPicture
  ),
  (
    "a picture is handed over with its sets in order and marked as referenced",
    testAPictureIsHandedOverWithItsSetsInOrderAndMarkedAsReferenced
  ),
  (
    "a picture with no sets and one with no bytes are handed over too",
    testAPictureWithNoSetsAndOneWithNoBytesAreHandedOverToo
  ),
  (
    "what became of a picture is told in the words of the library",
    testWhatBecameOfAPictureIsToldInTheWordsOfTheLibrary
  ),
  (
    "the end of a session goes with its sentence or with none",
    testTheEndOfASessionGoesWithItsSentenceOrWithNone
  ),
  (
    "a cancel reaches the library",
    testACancelReachesTheLibrary
  ),
  (
    "an image is made into the picture that the library asks for at the time it was captured",
    testAnImageIsMadeIntoThePictureThatTheLibraryAsksForAtTheTimeItWasCaptured
  ),
  (
    "a recovery frame is made as a key frame since nothing is marked as a long term reference",
    testARecoveryFrameIsMadeAsAKeyFrameSinceNothingIsMarkedAsALongTermReference
  ),
  (
    "an image for which no picture is wanted is kept and made when the frame clock ticks",
    testAnImageForWhichNoPictureIsWantedIsKeptAndMadeWhenTheFrameClockTicks
  ),
  (
    "only the newest image is made into a picture",
    testOnlyTheNewestImageIsMadeIntoAPicture
  ),
  (
    "an image that was sent is not made again unless the viewer needs something other than a delta",
    testAnImageThatWasSentIsNotMadeAgainUnlessTheViewerNeedsSomethingOtherThanADelta
  ),
  (
    "the frame clock does nothing before the first image comes",
    testTheFrameClockDoesNothingBeforeTheFirstImageComes
  ),
  (
    "a picture that is stale is asked for again on the next tick",
    testAPictureThatIsStaleIsAskedForAgainOnTheNextTick
  ),
  (
    "a picture that the encoder cannot make is given up on and the encoder is made anew",
    testAPictureThatTheEncoderCannotMakeIsGivenUpOnAndTheEncoderIsMadeAnew
  ),
  (
    "a session ends with a sentence when pictures fail too often in a row",
    testASessionEndsWithASentenceWhenPicturesFailTooOftenInARow
  ),
  (
    "a stale picture is not a failure",
    testAStalePictureIsNotAFailure
  ),
  (
    "a picture that works starts the count of failures again",
    testAPictureThatWorksStartsTheCountOfFailuresAgain
  ),
  (
    "an encoder that cannot be made anew ends the session",
    testAnEncoderThatCannotBeMadeAnewEndsTheSession
  ),
  (
    "pictures that are too large or not taken end the session when they repeat",
    testPicturesThatAreTooLargeOrNotTakenEndTheSessionWhenTheyRepeat
  ),
  (
    "a tick asks once about an image that was not made and not also about a lost picture",
    testATickAsksOnceAboutAnImageThatWasNotMadeAndNotAlsoAboutALostPicture
  ),
  (
    "a session that is over stops the making of pictures",
    testASessionThatIsOverStopsTheMakingOfPictures
  ),
  (
    "the image that was kept is let go of when the pump stops",
    testTheImageThatWasKeptIsLetGoOfWhenThePumpStops
  ),
  (
    "every begin is answered by one send or one cancel whatever happens",
    testEveryBeginIsAnsweredByOneSendOrOneCancelWhateverHappens
  ),
  (
    "the sentence for a stopped capture depends on why it stopped",
    testTheSentenceForAStoppedCaptureDependsOnWhyItStopped
  ),
  (
    "a code of another domain is never taken for one that the person caused",
    testACodeOfAnotherDomainIsNeverTakenForOneThatThePersonCaused
  ),
  (
    "every sentence for a stopped capture fits in the library's message",
    testEverySentenceForAStoppedCaptureFitsInTheLibrarysMessage
  ),
]

tests += [
  (
    "the surroundings of a window in front are on the screen with the keyboard",
    testTheSurroundingsOfAWindowInFrontAreOnTheScreenWithTheKeyboard
  ),
  (
    "a locked Mac is told without asking the window server",
    testALockedMacIsToldWithoutAskingTheWindowServer
  ),
  (
    "a window that is gone or not on the screen is not on the screen",
    testAWindowThatIsGoneOrNotOnTheScreenIsNotOnTheScreen
  ),
  (
    "a window server that cannot be asked makes no answer",
    testAWindowServerThatCannotBeAskedMakesNoAnswer
  ),
  (
    "windows in front are listed when they draw something and can lie over the window",
    testWindowsInFrontAreListedWhenTheyDrawSomethingAndCanLieOverTheWindow
  ),
  (
    "an open menu counts wherever it is and other windows only where they overlap",
    testAnOpenMenuCountsWhereverItIsAndOtherWindowsOnlyWhereTheyOverlap
  ),
  (
    "the keyboard is the window's only while its application is in front with no window of it before",
    testTheKeyboardIsTheWindowsOnlyWhileItsApplicationIsInFrontWithNoWindowOfItBefore
  ),
  (
    "more windows in front than the Rust side takes make no answer",
    testMoreWindowsInFrontThanTheRustSideTakesMakeNoAnswer
  ),
  (
    "the application is brought to the front once at the first touch before the windows in front are read",
    testTheApplicationIsBroughtToTheFrontOnceAtTheFirstTouchBeforeTheWindowsInFrontAreRead
  ),
  (
    "the first touch waits for the application for well less than a picture may be out of date",
    testTheFirstTouchWaitsForTheApplicationForWellLessThanAPictureMayBeOutOfDate
  ),
  (
    "the application is not brought forward when it is in front or when the Mac does not say",
    testTheApplicationIsNotBroughtForwardWhenItIsInFrontOrWhenTheMacDoesNotSay
  ),
  (
    "a window that is not on the screen brings nothing forward and the first touch after it is back does",
    testAWindowThatIsNotOnTheScreenBringsNothingForwardAndTheFirstTouchAfterItIsBackDoes
  ),
  (
    "a locked Mac is not touched and the first touch after the unlock brings the application",
    testALockedMacIsNotTouchedAndTheFirstTouchAfterTheUnlockBringsTheApplication
  ),
  (
    "what the watch looks at is the lock and whether the window is on the screen",
    testWhatTheWatchLooksAtIsTheLockAndWhetherTheWindowIsOnTheScreen
  ),
  (
    "the pause watch tells the library only when the answer changes",
    testThePauseWatchTellsTheLibraryOnlyWhenTheAnswerChanges
  ),
  (
    "the geometry of a picture holds the window and where it is in the picture",
    testTheGeometryOfAPictureHoldsTheWindowAndWhereItIsInThePicture
  ),
  ("a geometry that does not fit the picture is none", testAGeometryThatDoesNotFitThePictureIsNone),
  (
    "a rectangle that is not the window scaled to fit is not its geometry",
    testARectangleThatIsNotTheWindowScaledToFitIsNotItsGeometry
  ),
  (
    "a rectangle is read from the dictionary that the system gives",
    testARectangleIsReadFromTheDictionaryThatTheSystemGives
  ),
  (
    "the layout of an image is the window and what the capture says of it",
    testTheLayoutOfAnImageIsTheWindowAndWhatTheCaptureSaysOfIt
  ),
  (
    "when the capture says nothing that fits a window of the shape of the picture fills it",
    testWhenTheCaptureSaysNothingThatFitsAWindowOfTheShapeOfThePictureFillsIt
  ),
  (
    "when nothing fits the image has no layout and the numbers are told",
    testWhenNothingFitsTheImageHasNoLayoutAndTheNumbersAreTold
  ),
  (
    "the layout is told when it changes and only then", testTheLayoutIsToldWhenItChangesAndOnlyThen
  ),
  (
    "a window that the window server does not have has no layout",
    testAWindowThatTheWindowServerDoesNotHaveHasNoLayout
  ),
  (
    "a reading of the window server is used for less than its time and not for all of it",
    testAReadingOfTheWindowServerIsUsedForLessThanItsTimeAndNotForAllOfIt
  ),
  (
    "the window server is asked once for many images", testTheWindowServerIsAskedOnceForManyImages
  ),
  (
    "the layout of an image of a display is the display and what the capture says of it",
    testTheLayoutOfAnImageOfADisplayIsTheDisplayAndWhatTheCaptureSaysOfIt
  ),
  (
    "when the capture says nothing a display of the shape of the picture fills it",
    testWhenTheCaptureSaysNothingADisplayOfTheShapeOfThePictureFillsIt
  ),
  (
    "the layout of a display is told as a screen with the numbers and nothing more",
    testTheLayoutOfADisplayIsToldAsAScreenWithTheNumbersAndNothingMore
  ),
  (
    "a display that the system does not have has no layout until it comes back",
    testADisplayThatTheSystemDoesNotHaveHasNoLayoutUntilItComesBack
  ),
  (
    "the system is asked once for many images of a display",
    testTheSystemIsAskedOnceForManyImagesOfADisplay
  ),
  (
    "every key of a US keyboard has the code of its position on a Mac",
    testEveryKeyOfAUsKeyboardHasTheCodeOfItsPositionOnAMac
  ),
  (
    "the table of keys has no other keys and no two usages for one code",
    testTheTableOfKeysHasNoOtherKeysAndNoTwoUsagesForOneCode
  ),
  (
    "the modifier keys are the eight usages at the end of the page",
    testTheModifierKeysAreTheEightUsagesAtTheEndOfThePage
  ),
  ("the modifiers are the bits of the interface", testTheModifiersAreTheBitsOfTheInterface),
  (
    "a key goes down and comes up with the modifiers that are down",
    testAKeyGoesDownAndComesUpWithTheModifiersThatAreDown
  ),
  (
    "a key that is held repeats only while it goes down",
    testAKeyThatIsHeldRepeatsOnlyWhileItGoesDown
  ),
  (
    "a modifier key makes no event and a key with no code on a Mac makes none",
    testAModifierKeyMakesNoEventAndAKeyWithNoCodeOnAMacMakesNone
  ),
  (
    "the text of a key is left to the key and text with no key is typed once",
    testTheTextOfAKeyIsLeftToTheKeyAndTextWithNoKeyIsTypedOnce
  ),
  (
    "text of more than twenty units of UTF-16 is not typed",
    testTextOfMoreThanTwentyUnitsOfUtf16IsNotTyped
  ),
  (
    "the keys that a Mac keyboard gives flags carry them and no other key does",
    testTheKeysThatAMacKeyboardGivesFlagsCarryThemAndNoOtherKeyDoes
  ),
  (
    "the shortcuts that the system keeps for itself are not let through to a window",
    testTheShortcutsThatTheSystemKeepsForItselfAreNotLetThroughToAWindow
  ),
  (
    "the keys of the application go through to a window as they do with no rule",
    testTheKeysOfTheApplicationGoThroughToAWindowAsTheyDoWithNoRule
  ),
  (
    "nothing is left out unless the translator is told to",
    testNothingIsLeftOutUnlessTheTranslatorIsToldTo
  ),
  (
    "the release of a key that was left out is left out and no other is",
    testTheReleaseOfAKeyThatWasLeftOutIsLeftOutAndNoOtherIs
  ),
  (
    "a key that went through is released when its repeat becomes a shortcut",
    testAKeyThatWentThroughIsReleasedWhenItsRepeatBecomesAShortcut
  ),
  ("text with no key is not a shortcut", testTextWithNoKeyIsNotAShortcut),
  (
    "a window does not get a shortcut of the system and counts it as not done",
    testAWindowDoesNotGetAShortcutOfTheSystemAndCountsItAsNotDone
  ),
  ("a pointer moves presses drags and releases", testAPointerMovesPressesDragsAndReleases),
  ("the buttons are their own", testTheButtonsAreTheirOwn),
  (
    "with two buttons down the first one drags and a release lets go of only its own button",
    testWithTwoButtonsDownTheFirstOneDragsAndAReleaseLetsGoOfOnlyItsOwnButton
  ),
  (
    "presses count as clicks when they come soon and close to each other",
    testPressesCountAsClicksWhenTheyComeSoonAndCloseToEachOther
  ),
  (
    "clicks in a row go on counting until one comes too late",
    testClicksInARowGoOnCountingUntilOneComesTooLate
  ),
  (
    "a drag and the release of a click carry its count",
    testADragAndTheReleaseOfAClickCarryItsCount
  ),
  (
    "a pointer at a place that is not a number makes no event",
    testAPointerAtAPlaceThatIsNotANumberMakesNoEvent
  ),
  (
    "a scroll moves by whole pixels and keeps what is left",
    testAScrollMovesByWholePixelsAndKeepsWhatIsLeft
  ),
  ("a scroll is bounded and must be a number", testAScrollIsBoundedAndMustBeANumber),
  (
    "a scroll for the system is preceded by a move to its place",
    testAScrollForTheSystemIsPrecededByAMoveToItsPlace
  ),
  (
    "a scroll is never preceded by a move unless the translator is told to",
    testAScrollIsNeverPrecededByAMoveUnlessTheTranslatorIsToldTo
  ),
  (
    "a scroll that moves nothing does not move the pointer and one with a button down drags",
    testAScrollThatMovesNothingDoesNotMoveThePointerAndOneWithAButtonDownDrags
  ),
  (
    "a window is steered by handing each event to its poster",
    testAWindowIsSteeredByHandingEachEventToItsPoster
  ),
  (
    "an event that cannot be handed over is a failure and the rest of the post is not tried",
    testAnEventThatCannotBeHandedOverIsAFailureAndTheRestOfThePostIsNotTried
  ),
  (
    "a display is steered by asking the display and handing each event to its poster",
    testADisplayIsSteeredByAskingTheDisplayAndHandingEachEventToItsPoster
  ),
  (
    "the surroundings of a display are its place with the keyboard and nothing in front",
    testTheSurroundingsOfADisplayAreItsPlaceWithTheKeyboardAndNothingInFront
  ),
  (
    "a display left of the main one keeps its place below zero",
    testADisplayLeftOfTheMainOneKeepsItsPlaceBelowZero
  ),
  (
    "a locked Mac is told without asking for the display",
    testALockedMacIsToldWithoutAskingForTheDisplay
  ),
  (
    "a display that is not there is not on the screen", testADisplayThatIsNotThereIsNotOnTheScreen
  ),
  (
    "a locked Mac is told with the last place that the display had",
    testALockedMacIsToldWithTheLastPlaceThatTheDisplayHad
  ),
  (
    "a display that is gone is told with the last place that it had",
    testADisplayThatIsGoneIsToldWithTheLastPlaceThatItHad
  ),
  ("the table has the two functions of input", testTheTableHasTheTwoFunctionsOfInput),
  (
    "the consent is told to the app with the words it reads",
    testTheConsentIsToldToTheAppWithTheWordsItReads
  ),
  (
    "the consent to record the screen is told to the app with the words it reads",
    testTheConsentToRecordTheScreenIsToldToTheAppWithTheWordsItReads
  ),
  (
    "a rectangle can hold something only with a finite size above zero",
    testARectangleCanHoldSomethingOnlyWithAFiniteSizeAboveZero
  ),
  ("a rectangle ends where its size takes it", testARectangleEndsWhereItsSizeTakesIt),
  (
    "two rectangles overlap only when they share some area",
    testTwoRectanglesOverlapOnlyWhenTheyShareSomeArea
  ),
  (
    "the surroundings of a window are written where the Rust side left room for them",
    testTheSurroundingsOfAWindowAreWrittenWhereTheRustSideLeftRoomForThem
  ),
  (
    "surroundings that cannot be said or do not fit are a failure and leave the struct alone",
    testSurroundingsThatCannotBeSaidOrDoNotFitAreAFailureAndLeaveTheStructAlone
  ),
  (
    "surroundings are refused for a struct that is too short or a window that is not there",
    testSurroundingsAreRefusedForAStructThatIsTooShortOrAWindowThatIsNotThere
  ),
  (
    "a pointer that the Rust side puts to the window is read as it was meant",
    testAPointerThatTheRustSidePutsToTheWindowIsReadAsItWasMeant
  ),
  (
    "a key that the Rust side puts to the window is read as it was meant",
    testAKeyThatTheRustSidePutsToTheWindowIsReadAsItWasMeant
  ),
  (
    "a post that is not well formed is refused and never reaches the window",
    testAPostThatIsNotWellFormedIsRefusedAndNeverReachesTheWindow
  ),
  (
    "a window that cannot be steered says nothing and puts nothing",
    testAWindowThatCannotBeSteeredSaysNothingAndPutsNothing
  ),
  (
    "a picture carries the geometry of its window to the library",
    testAPictureCarriesTheGeometryOfItsWindowToTheLibrary
  ),
  (
    "a change a pause and a resume reach the library about the link",
    testAChangeAPauseAndAResumeReachTheLibraryAboutTheLink
  ),
  (
    "every image the capture hands over is told as a change before a picture is asked for",
    testEveryImageTheCaptureHandsOverIsToldAsAChangeBeforeAPictureIsAskedFor
  ),
  ("no change is told once the session is over", testNoChangeIsToldOnceTheSessionIsOver),
  (
    "a picture has the geometry of the image it was made from",
    testAPictureHasTheGeometryOfTheImageItWasMadeFrom
  ),
  (
    "a picture that the encoder made keeps its own fields",
    testAPictureThatTheEncoderMadeKeepsItsOwnFields
  ),
  (
    "the cover goes up once and a failure leaves nothing raised",
    testTheCoverGoesUpOnceAndAFailureLeavesNothingRaised
  ),
  (
    "a window that was only given and let go of raises nothing and cannot be raised afterwards",
    testAWindowThatWasOnlyGivenAndLetGoOfRaisesNothingAndCannotBeRaisedAfterwards
  ),
  (
    "the cover comes down only after the Mac is locked and the lock screen was drawn",
    testTheCoverComesDownOnlyAfterTheMacIsLockedAndTheLockScreenWasDrawn
  ),
  (
    "a Mac that is locked already is not locked again and keeps the cover until the end",
    testAMacThatIsLockedAlreadyIsNotLockedAgainAndKeepsTheCoverUntilTheEnd
  ),
  (
    "a Mac that does not lock the first way is tried the next ways and keeps its cover",
    testAMacThatDoesNotLockWithTheFirstWayIsTriedWithTheNextOnesAndKeepsItsCoverMeanwhile
  ),
  (
    "a Mac that cannot be locked keeps its cover and finish returns after the patience",
    testAMacThatCannotBeLockedKeepsItsCoverAndFinishReturnsAfterThePatience
  ),
  (
    "something at the Mac ends the session once, tells the other device and locks the Mac",
    testSomethingAtTheMacEndsTheSessionOnceTellsTheOtherDeviceAndLocksTheMac
  ),
  (
    "something at the Mac while the cover goes up ends the session after the cover is up",
    testSomethingThatHappensAtTheMacWhileTheCoverGoesUpEndsTheSessionAfterTheCoverIsUp
  ),
  (
    "the Mac is locked while the other device is still being told",
    testTheMacIsLockedWhileTheOtherDeviceIsStillBeingToldAndNeitherWaitsForTheOther
  ),
  (
    "an alarm does not hold up whoever rings it, and the cover stays until the Mac is locked",
    testAnAlarmDoesNotHoldUpWhoeverRingsItAndTheCoverStaysUntilTheMacIsLocked
  ),
  (
    "the end of a session waits for the lock and a release that follows does not",
    testTheEndOfASessionWaitsForTheLockAndAReleaseThatFollowsDoesNot
  ),
  (
    "a session that did not start after the cover went up is locked when it is released",
    testASessionThatDidNotStartAfterTheCoverWentUpIsLockedAndGivenBackWhenItIsReleased
  ),
  (
    "what is lifted reaches the curtain only while the cover is up and each call replaces the last",
    testWhatIsLiftedReachesTheCurtainOnlyWhileTheCoverIsUpAndEachCallReplacesTheOneBefore
  ),
  (
    "a session that ends with something lifted puts it back before it locks the Mac",
    testASessionThatEndsWithSomethingLiftedPutsItBackBeforeItLocksTheMac
  ),
  (
    "a part that cannot be put back ends the session and the Mac is locked anyway",
    testAPartThatCannotBePutBackEndsTheSessionTellsTheOtherDeviceAndLocksTheMacAnyway
  ),
  (
    "a finish that comes while something is being lifted waits for the lift",
    testAFinishThatComesWhileSomethingIsBeingLiftedWaitsForTheLiftAndThenPutsItBack
  ),
  (
    "the guard is handed the Mac before the cover goes up and let go of after it comes down",
    testTheGuardIsHandedTheMacBeforeTheCoverGoesUpAndLetGoOfAfterItComesDown
  ),
  (
    "a guard that cannot be had raises nothing and says why",
    testAGuardThatCannotBeHadRaisesNothingAndSaysWhy
  ),
  (
    "a cover that does not go up gives the guard back and locks nothing",
    testACoverThatDoesNotGoUpGivesTheGuardBackAndLocksNothing
  ),
  (
    "a guard that is lost ends the session, tells the other device and locks the Mac",
    testAGuardThatIsLostEndsTheSessionTellsTheOtherDeviceAndLocksTheMac
  ),
  (
    "a guard that is lost while the cover goes up ends the session after the cover is up",
    testAGuardThatIsLostWhileTheCoverGoesUpEndsTheSessionAfterTheCoverIsUp
  ),
  (
    "the guard is told that the session is well only while the curtain is",
    testTheGuardIsToldThatTheSessionIsWellOnlyWhileTheCurtainIs
  ),
  (
    "the guard is told that the app is at work while the cover goes up",
    testTheGuardIsToldThatTheAppIsAtWorkWhileTheCoverGoesUp
  ),
  (
    "the guard is not told that the app is at work for longer than the bound",
    testTheGuardIsNotToldThatTheAppIsAtWorkForLongerThanTheBound
  ),
  (
    "the guard is told until the Mac is locked and the cover is down",
    testTheGuardIsToldUntilTheMacIsLockedAndTheCoverIsDown
  ),
  (
    "a session that nobody holds does not go on beating",
    testASessionThatNobodyHoldsDoesNotGoOnBeating
  ),
  (
    "a quit of the app locks the Mac before it goes on",
    testAQuitOfTheAppLocksTheMacBeforeItGoesOn
  ),
  (
    "a watch of the quit calls its action and stops when it is let go of",
    testAWatchOfTheQuitCallsItsActionOnTheThreadThatQuitsAndStopsWhenItIsLetGoOf
  ),
  (
    "three presses of Escape in two seconds are the way out",
    testThreePressesOfEscapeInTwoSecondsAreTheWayOut
  ),
  (
    "a key that the app did not post is found and the keys of the app are not",
    testAKeyThatTheAppDidNotPostIsFoundAndTheKeysOfTheAppAreNot
  ),
  (
    "the ledger says how long ago the app last posted a key",
    testTheLedgerSaysHowLongAgoTheAppLastPostedAKey
  ),
  (
    "what the other device is told is a short sentence that says the Mac locks",
    testWhatTheOtherDeviceIsToldIsAShortSentenceThatSaysTheMacLocks
  ),
  (
    "the guard locks the Mac when the app is gone after it was handed the Mac",
    testTheGuardLocksWhenTheAppIsGoneAfterItWasHandedTheMac
  ),
  (
    "an armed guard locks the Mac when no line comes for the lapse",
    testAnArmedGuardLocksWhenNoLineComesForTheLapse
  ),
  ("beats keep an armed guard from locking", testBeatsKeepAnArmedGuardFromLocking),
  (
    "the guard ends without locking when the session ends cleanly",
    testTheGuardEndsWithoutLockingWhenTheSessionEndsCleanly
  ),
  (
    "a guard that was never handed the Mac locks nothing",
    testAGuardThatWasNeverHandedTheMacLocksNothing
  ),
  ("an app that arms twice is answered twice", testAnAppThatArmsTwiceIsAnsweredTwice),
  (
    "lines are cut out of whatever comes through a pipe",
    testLinesAreCutOutOfWhateverComesThroughAPipe
  ),
  (
    "locking goes round the ways until the Mac is locked",
    testLockingGoesRoundTheWaysUntilTheMacIsLocked
  ),
  (
    "a session beats several times in the lapse of its guard",
    testASessionBeatsSeveralTimesInTheLapseOfItsGuard
  ),
  (
    "a guard says it is ready, is handed the Mac and ends when the session ends",
    testAGuardSaysItIsReadyIsHandedTheMacAndEndsWhenTheSessionEnds
  ),
  (
    "a guard locks the Mac when its input closes after it was handed the Mac",
    testAGuardLocksTheMacWhenItsInputClosesAfterItWasHandedTheMac
  ),
  (
    "a guard locks the Mac when the app stops saying that it is well",
    testAGuardLocksTheMacWhenTheAppStopsSayingThatItIsWell
  ),
  (
    "beats keep a guard from locking for as long as they come and only then",
    testBeatsKeepAGuardFromLockingForAsLongAsTheyComeAndOnlyThen
  ),
  (
    "a guard that was never handed the Mac ends and locks nothing",
    testAGuardThatWasNeverHandedTheMacEndsAndLocksNothing
  ),
  (
    "a termination locks the Mac of a guard that was handed it",
    testATerminationLocksTheMacOfAGuardThatWasHandedIt
  ),
  (
    "a guard keeps trying to lock the Mac until it is locked",
    testAGuardKeepsTryingToLockTheMacUntilItIsLocked
  ),
  ("a guard ignores lines it does not know", testAGuardIgnoresLinesItDoesNotKnow),
  (
    "a guard whose parent changes locks the Mac even if its input stays open",
    testAGuardWhoseParentChangesLocksTheMacEvenIfItsInputStaysOpen
  ),
  (
    "an app that is killed leaves a guard that locks the Mac",
    testAnAppThatIsKilledLeavesAGuardThatLocksTheMac
  ),
  (
    "an app that ends the session leaves a guard that locks nothing",
    testAnAppThatEndsTheSessionLeavesAGuardThatLocksNothing
  ),
  (
    "an app that hangs leaves a guard that locks the Mac when the lapse is over",
    testAnAppThatHangsLeavesAGuardThatLocksTheMacWhenTheLapseIsOver
  ),
  (
    "a guard that is terminated while it holds the Mac locks it and the app is told",
    testAGuardThatIsTerminatedWhileItHoldsTheMacLocksIt
  ),
  (
    "a guard that is killed while it holds the Mac is reported to the app",
    testAGuardThatIsKilledWhileItHoldsTheMacIsReportedToTheApp
  ),
  (
    "a guard whose pipe breaks while it stays is reported to the app",
    testAGuardWhosePipeBreaksWhileItStaysIsReportedToTheApp
  ),
  (
    "a guard that is not there or does not answer is not armed",
    testAGuardThatIsNotThereOrDoesNotAnswerIsNotArmed
  ),
  (
    "an arm that is answered too late is taken back and locks nothing",
    testAnArmThatIsAnsweredTooLateIsTakenBackAndLocksNothing
  ),
  (
    "a link is armed once and the guard ends when it is disarmed",
    testALinkIsArmedOnceAndTheGuardEndsWhenItIsDisarmed
  ),
  (
    "listening starts the tap muted and begins the run in the form of the tap",
    testListeningStartsTheTapMutedAndBeginsTheRunInTheFormOfTheTap
  ),
  (
    "samples become packets of 1024 frames with the time they were recorded",
    testSamplesBecomePacketsOfOneThousandAndTwentyFourFramesWithTheTimeTheyWereRecorded
  ),
  (
    "not listening any more stops the tap and blocks that came late are not sent",
    testNotListeningAnyMoreStopsTheTapAndBlocksThatCameLateAreNotSent
  ),
  (
    "a quiet screen keeps the Mac quiet for the whole session and sends only what is wanted",
    testAQuietScreenKeepsTheMacQuietForTheWholeSessionAndSendsOnlyWhatIsWanted
  ),
  (
    "listening again with no stop between begins another run from nothing",
    testListeningAgainWithNoStopBetweenBeginsAnotherRunFromNothing
  ),
  (
    "two seconds of silence while a program plays are taken for a Mac that has not let the app record",
    testTwoSecondsOfSilenceWhileAProgramPlaysAreTakenForAMacThatHasNotLetTheAppRecord
  ),
  (
    "silence while nothing plays is not reported and the count starts again after each look",
    testSilenceWhileNothingPlaysIsNotReportedAndTheCountStartsAgainAfterEachLook
  ),
  (
    "sound that was heard once rests the question of the permission",
    testSoundThatWasHeardOnceRestsTheQuestionOfThePermission
  ),
  (
    "a tap or an encoder that cannot be made is told as failed and the Mac plays again",
    testATapOrAnEncoderThatCannotBeMadeIsToldAsFailedAndTheMacPlaysAgain
  ),
  (
    "an encoder that fails ends the run as failed and one with no packet yet skips one",
    testAnEncoderThatFailsEndsTheRunAsFailedAndOneWithNoPacketYetSkipsOne
  ),
  (
    "packets that are dropped or too large or refused go on and over ends the run",
    testPacketsThatAreDroppedOrTooLargeOrRefusedGoOnAndOverEndsTheRun
  ),
  (
    "a clock that is gone ends the run without saying why",
    testAClockThatIsGoneEndsTheRunWithoutSayingWhy
  ),
  (
    "close is final and lets go of the tap and the encoder once",
    testCloseIsFinalAndLetsGoOfTheTapAndTheEncoderOnce
  ),
  (
    "the rate of the session is that of the Mac when it is 44100 or 48000 and any other is 48000",
    testTheRateOfTheSessionIsThatOfTheMacWhenItIs44100Or48000AndAnyOtherIs48000
  ),
  (
    "a second run keeps the form of the session when the Mac changes its rate",
    testASecondRunKeepsTheFormOfTheSessionWhenTheMacChangesItsRate
  ),
  (
    "the resampler keeps a line straight across blocks of any size",
    testTheResamplerKeepsALineStraightAcrossBlocksOfAnySize
  ),
  (
    "the resampler needs rates above zero and keeps the same rate whole",
    testTheResamplerNeedsRatesAboveZeroAndKeepsTheSameRateWhole
  ),
  (
    "the library session makes the calls of the interface with what it was given",
    testTheLibrarySessionMakesTheCallsOfTheInterfaceWithWhatItWasGiven
  ),
  (
    "the library session tells what became of every call",
    testTheLibrarySessionTellsWhatBecameOfEveryCall
  ),
  (
    "the calls of the sound may come from many threads at once",
    testTheCallsOfTheSoundMayComeFromManyThreadsAtOnce
  ),
]

tests += [
  (
    "the ring plays nothing until it holds the start level and then plays in order",
    testTheRingPlaysNothingUntilItHoldsTheStartLevelAndThenPlaysInOrder
  ),
  (
    "the ring plays silence and fills again when it runs dry",
    testTheRingPlaysSilenceAndFillsAgainWhenItRunsDry
  ),
  (
    "the ring drops the oldest sound when it would hold more than the most",
    testTheRingDropsTheOldestSoundWhenItWouldHoldMoreThanTheMost
  ),
  (
    "the ring lets go of sound that it did not come down to in a second of playing",
    testTheRingLetsGoOfSoundThatItDidNotComeDownToInASecondOfPlaying
  ),
  (
    "the ring keeps what the level came down to within the slack and lets go in the next second",
    testTheRingKeepsWhatTheLevelCameDownToWithinTheSlackAndLetsGoInTheNextSecond
  ),
  (
    "the ring begins the second again when it runs dry",
    testTheRingBeginsTheSecondAgainWhenItRunsDry
  ),
  (
    "the ring keeps the order of sound that wraps around its storage",
    testTheRingKeepsTheOrderOfSoundThatWrapsAroundItsStorage
  ),
  (
    "a gap is covered with silence only while the ring plays with sound in it",
    testAGapIsCoveredWithSilenceOnlyWhileTheRingPlaysWithSoundInIt
  ),
  (
    "the ring takes mono and ignores a part of a frame at the end",
    testTheRingTakesMonoAndIgnoresAPartOfAFrameAtTheEnd
  ),
  (
    "a ring that one thread writes and another reads keeps the order and loses nothing unaccounted for",
    testARingThatOneThreadWritesAndAnotherReadsKeepsTheOrderAndLosesNothingUnaccountedFor
  ),
  (
    "a run begins with a decoder, an output and a ring of its form",
    testARunBeginsWithADecoderAnOutputAndARingOfItsForm
  ),
  (
    "packets are decoded in order and played once the ring holds the start level",
    testPacketsAreDecodedInOrderAndPlayedOnceTheRingHoldsTheStartLevel
  ),
  (
    "the first packet of a run and the packet after a gap reset the decoder",
    testTheFirstPacketOfARunAndThePacketAfterAGapResetTheDecoder
  ),
  (
    "a speaker covers a gap with silence while it plays",
    testASpeakerCoversAGapWithSilenceWhileItPlays
  ),
  (
    "a packet that cannot be decoded is dropped and counts as missing",
    testAPacketThatCannotBeDecodedIsDroppedAndCountsAsMissing
  ),
  (
    "a packet that gives no samples changes nothing",
    testAPacketThatGivesNoSamplesChangesNothing
  ),
  (
    "a packet before a run begins and after it ended is dropped",
    testAPacketBeforeARunBeginsAndAfterItEndedIsDropped
  ),
  (
    "stopped ends the run, lets go of the decoder and stops the output once",
    testStoppedEndsTheRunLetsGoOfTheDecoderAndStopsTheOutputOnce
  ),
  ("a new run ends the one that went on", testANewRunEndsTheOneThatWentOn),
  (
    "closing ends the run and later calls do nothing",
    testClosingEndsTheRunAndLaterCallsDoNothing
  ),
  (
    "a decoder or an output that cannot be made ends the run quietly",
    testADecoderOrAnOutputThatCannotBeMadeEndsTheRunQuietly
  ),
  (
    "a run that could not be made is tried again on a packet after a second",
    testARunThatCouldNotBeMadeIsTriedAgainOnAPacketAfterASecond
  ),
  (
    "a run that keeps failing is tried again once per second",
    testARunThatKeepsFailingIsTriedAgainOncePerSecond
  ),
  (
    "a run stopped before it was made is not tried again, and a new run takes its place",
    testARunThatWasStoppedBeforeItWasMadeIsNotTriedAgainAndANewOneIsTriedInItsForm
  ),
  ("the packet is copied before play returns", testThePacketIsCopiedBeforePlayReturns),
  (
    "closing does not wait for the output to start and stops it afterwards",
    testClosingDoesNotWaitForTheOutputToStartAndStopsItAfterwards
  ),
  (
    "a window screen hands the sound to its speaker and closes it when it ends",
    testAWindowScreenHandsTheSoundToItsSpeakerAndClosesItWhenItEnds
  ),
  (
    "a screen with no speaker takes the calls about sound and does nothing",
    testAScreenWithNoSpeakerTakesTheCallsAboutSoundAndDoesNothing
  ),
  (
    "a platform that plays sound gives the three entries and the screen is told in order",
    testAPlatformThatPlaysSoundGivesTheThreeEntriesAndTheScreenIsToldInOrder
  ),
  (
    "sound calls with structs that are not whole never reach the screen",
    testSoundCallsWithStructsThatAreNotWholeNeverReachTheScreen
  ),
  (
    "a screen that plays no sound is not given calls about it",
    testAScreenThatPlaysNoSoundIsNotGivenCallsAboutIt
  ),
]

#if canImport(AudioToolbox)
  tests += [
    (
      "the decoder makes the tone back that the encoder of the system made of it",
      testTheDecoderMakesTheToneBackThatTheEncoderOfTheSystemMadeOfIt
    ),
    (
      "the decoder makes a tone of one channel at the rate of CDs too",
      testTheDecoderMakesAToneOfOneChannelAtTheRateOfCdsToo
    ),
    (
      "the decoder goes on after a packet that is not audio and a reset",
      testTheDecoderGoesOnAfterAPacketThatIsNotAudioAndAReset
    ),
    (
      "the decoder is not made for a form that is not AAC-LC of one or two channels",
      testTheDecoderIsNotMadeForAFormThatIsNotAacLcOfOneOrTwoChannels
    ),
    (
      "the description of the stream is what the standard writes",
      testTheDescriptionOfTheStreamIsWhatTheStandardWritesAndNothingForWhatIsNotAacLc
    ),
  ]
#endif

#if canImport(AudioToolbox)
  tests += [
    ("the AAC encoder makes packets of a tone", testTheAacEncoderMakesPacketsOfATone),
    (
      "the AAC encoder refuses what it cannot make and is over when let go of",
      testTheAacEncoderRefusesWhatItCannotMakeAndIsOverWhenLetGoOf
    ),
    (
      "the decoder makes the tone back that the encoder of the Mac made of it",
      testTheDecoderMakesTheToneBackThatTheEncoderOfTheMacMadeOfIt
    ),
  ]
#endif

#if canImport(AVFoundation)
  tests += [
    (
      "the samples of an output go to buffers of one channel each and to one with the channels interleaved",
      testTheSamplesOfAnOutputGoToBuffersOfOneChannelEachAndToOneWithTheChannelsInterleaved
    )
  ]
#endif

#if canImport(VideoToolbox)
  tests += [
    (
      "VideoToolbox gives back the samples that were coded",
      testVideoToolboxGivesBackTheSamplesThatWereCoded
    ),
    (
      "VideoToolbox goes on when the same parameter sets come again",
      testVideoToolboxGoesOnWhenTheSameParameterSetsComeAgain
    ),
    ("VideoToolbox starts again for another size", testVideoToolboxStartsAgainForAnotherSize),
    (
      "VideoToolbox makes the format that Flutter draws",
      testVideoToolboxMakesTheFormatThatFlutterDraws
    ),
    (
      "VideoToolbox answers every frame that it took",
      testVideoToolboxAnswersEveryFrameThatItTook
    ),
    ("VideoToolbox refuses what it cannot use", testVideoToolboxRefusesWhatItCannotUse),
    (
      "VideoToolbox pictures reach the screen through the whole chain",
      testVideoToolboxPicturesReachTheScreenThroughTheWholeChain
    ),
    (
      "a picture with nothing in it is one that Flutter draws",
      testAPictureWithNothingInItIsOneThatFlutterDraws
    ),
    (
      "the encoder makes a key frame with its parameter sets and the frames after it without them",
      testTheEncoderMakesAKeyFrameWithItsParameterSetsAndTheFramesAfterItWithoutThem
    ),
    (
      "the encoder makes a key frame whenever one is asked for",
      testTheEncoderMakesAKeyFrameWheneverOneIsAskedFor
    ),
    ("two pictures of the same moment are both made", testTwoPicturesOfTheSameMomentAreBothMade),
    (
      "an encoder that is started anew makes a key frame and one let go of refuses to encode",
      testAnEncoderThatIsStartedAnewMakesAKeyFrameAndOneLetGoOfRefusesToEncode
    ),
    (
      "an encoder is not made for a size that the protocol does not allow",
      testAnEncoderIsNotMadeForASizeThatTheProtocolDoesNotAllow
    ),
    (
      "the decoder of the viewer takes what the encoder makes",
      testTheDecoderOfTheViewerTakesWhatTheEncoderMakes
    ),
    (
      "the pump makes pictures of images with the encoder of the system",
      testThePumpMakesPicturesOfImagesWithTheEncoderOfTheSystem
    ),
  ]
#endif

#if os(macOS)
  tests += [
    (
      "the Mac gives its main screen or says why it cannot",
      testTheMacGivesItsMainScreenOrSaysWhyItCannot
    ),
    (
      "a private screen is given only to an app that can keep it private and changes nothing until it starts",
      testAPrivateScreenIsGivenOnlyToAnAppThatCanKeepItPrivateAndChangesNothingUntilItStarts
    ),
  ]
#endif

for (name, body) in tests {
  let before = failures
  body()
  print("\(failures == before ? "ok  " : "FAIL") \(name)")
}
print("\(tests.count) tests, \(checks) checks, \(failures) failures")
exit(failures == 0 ? 0 : 1)
